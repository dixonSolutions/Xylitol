//! The download engine: resume, progress, verification and cancellation.
//!
//! These run against a throwaway HTTP server in-process rather than against
//! APKPure. That keeps them hermetic and fast, lets them assert things the real
//! site cannot be made to do on demand — a corrupt file, a stall, a client that
//! gives up halfway — and means they run in CI without rate limits.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha1::{Digest, Sha1};
use xylitol_apkpure::{CancelToken, Client, Error, FileKind, Progress, Variant};

/// A minimal HTTP/1.1 server that understands `Range: bytes=N-`.
struct Server {
    port: u16,
    /// Every `Range` header value received, in order.
    ranges: Arc<Mutex<Vec<Option<String>>>>,
    requests: Arc<AtomicUsize>,
}

impl Server {
    fn serve(body: Vec<u8>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let port = listener.local_addr().unwrap().port();
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(AtomicUsize::new(0));

        let body = Arc::new(body);
        let thread_ranges = ranges.clone();
        let thread_requests = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let body = body.clone();
                let ranges = thread_ranges.clone();
                thread_requests.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let _ = handle(stream, &body, &ranges);
                });
            }
        });

        Server {
            port,
            ranges,
            requests,
        }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/app.apk", self.port)
    }

    fn ranges_seen(&self) -> Vec<Option<String>> {
        self.ranges.lock().unwrap().clone()
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

fn handle(
    mut stream: TcpStream,
    body: &[u8],
    ranges: &Mutex<Vec<Option<String>>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    let range = headers.get("range").cloned();
    ranges.lock().unwrap().push(range.clone());

    // Only the open-ended `bytes=N-` form is used by the client.
    let start = range
        .as_deref()
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.strip_suffix('-'))
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|start| *start < body.len());

    let slice = match start {
        Some(start) => &body[start..],
        None => body,
    };

    let mut head = String::new();
    match start {
        Some(start) => {
            head.push_str("HTTP/1.1 206 Partial Content\r\n");
            head.push_str(&format!(
                "Content-Range: bytes {}-{}/{}\r\n",
                start,
                body.len() - 1,
                body.len()
            ));
        }
        None => head.push_str("HTTP/1.1 200 OK\r\n"),
    }
    head.push_str("Content-Type: application/vnd.android.package-archive\r\n");
    head.push_str(&format!("Content-Length: {}\r\n", slice.len()));
    head.push_str("Accept-Ranges: bytes\r\n\r\n");
    stream.write_all(head.as_bytes())?;

    // Dribble the body out so the client sees several chunks, which is what
    // makes progress reporting and mid-flight cancellation observable.
    for piece in slice.chunks(64 * 1024) {
        stream.write_all(piece)?;
        stream.flush()?;
    }
    Ok(())
}

/// Deterministic pseudo-random bytes: compressible data would be reshaped by
/// any transfer encoding and make byte counts harder to reason about.
fn payload(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut state = 0x243f_6a88_85a3_08d3u64;
    while out.len() < len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

fn sha1_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn variant(url: &str, size: u64, sha1: Option<String>) -> Variant {
    Variant {
        package: "com.example.app".into(),
        version_name: "1.0".into(),
        version_code: 7,
        kind: FileKind::Apk,
        size: Some(size),
        published: None,
        arch: Some("arm64-v8a".into()),
        dpi: None,
        min_android: None,
        sha1,
        signature: None,
        uploader: None,
        download_url: url.to_string(),
    }
}

#[tokio::test]
async fn a_download_is_written_verified_and_renamed() {
    let body = payload(600 * 1024);
    let server = Server::serve(body.clone());
    let dir = tempfile::tempdir().unwrap();

    let variant = variant(&server.url(), body.len() as u64, Some(sha1_hex(&body)));
    let mut updates = Vec::new();
    let done = Client::new()
        .download(&variant, dir.path(), &CancelToken::new(), |p: Progress| {
            updates.push(p.downloaded)
        })
        .await
        .expect("download should succeed");

    assert_eq!(done.bytes, body.len() as u64);
    assert!(done.verified, "a matching published checksum should verify");
    assert_eq!(done.sha1, sha1_hex(&body));
    assert_eq!(std::fs::read(&done.path).unwrap(), body);

    // The name encodes the choice, and nothing partial is left behind.
    assert_eq!(
        done.path.file_name().unwrap(),
        "com.example.app_1.0_7_arm64-v8a.apk"
    );
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")),
        "a .part file survived a successful download"
    );

    // Progress must start at zero, never go backwards, and finish at the total.
    assert!(
        updates.len() > 2,
        "progress was reported {} times",
        updates.len()
    );
    assert_eq!(updates.first().copied(), Some(0));
    assert_eq!(updates.last().copied(), Some(body.len() as u64));
    assert!(
        updates.windows(2).all(|w| w[0] <= w[1]),
        "progress went backwards"
    );
}

#[tokio::test]
async fn a_corrupt_download_is_rejected_and_not_left_to_resume() {
    let body = payload(200 * 1024);
    let server = Server::serve(body.clone());
    let dir = tempfile::tempdir().unwrap();

    // Claim a checksum the bytes will not match.
    let variant = variant(
        &server.url(),
        body.len() as u64,
        Some(sha1_hex(b"something else")),
    );
    let error = Client::new()
        .download(&variant, dir.path(), &CancelToken::new(), |_| {})
        .await
        .expect_err("a checksum mismatch must fail");

    assert!(
        matches!(error, Error::ChecksumMismatch { .. }),
        "got {error:?}"
    );
    // Keeping the bad bytes would let a resume turn them into permanent
    // corruption, and leave a file that looks installable.
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "the corrupt file was kept"
    );
}

#[tokio::test]
async fn an_unverifiable_download_is_kept_but_marked() {
    let body = payload(100 * 1024);
    let server = Server::serve(body.clone());
    let dir = tempfile::tempdir().unwrap();

    // No published checksum: common on APKPure, and not a reason to refuse.
    let variant = variant(&server.url(), body.len() as u64, None);
    let done = Client::new()
        .download(&variant, dir.path(), &CancelToken::new(), |_| {})
        .await
        .expect("a download without a checksum should still succeed");

    assert!(!done.verified, "nothing was verified against");
    assert_eq!(std::fs::read(&done.path).unwrap(), body);
}

#[tokio::test]
async fn an_interrupted_download_resumes_where_it_stopped() {
    let body = payload(900 * 1024);
    let server = Server::serve(body.clone());
    let dir = tempfile::tempdir().unwrap();
    let variant = variant(&server.url(), body.len() as u64, Some(sha1_hex(&body)));

    // First attempt: stop as soon as anything has arrived.
    let cancel = CancelToken::new();
    let stopper = cancel.clone();
    let error = Client::new()
        .download(&variant, dir.path(), &cancel, move |p: Progress| {
            if p.downloaded > 0 {
                stopper.cancel();
            }
        })
        .await
        .expect_err("the download was cancelled");
    assert!(matches!(error, Error::Cancelled), "got {error:?}");

    let part = dir.path().join("com.example.app_1.0_7_arm64-v8a.apk.part");
    let partial_len = std::fs::metadata(&part)
        .expect("the partial file is kept")
        .len();
    assert!(
        partial_len > 0 && partial_len < body.len() as u64,
        "partial was {partial_len}"
    );

    // Second attempt: must ask for the rest, not start over.
    let done = Client::new()
        .download(&variant, dir.path(), &CancelToken::new(), |_| {})
        .await
        .expect("resuming should succeed");

    assert_eq!(server.request_count(), 2);
    assert_eq!(
        server.ranges_seen()[1],
        Some(format!("bytes={partial_len}-")),
        "the second request did not resume from the partial file"
    );

    // The checksum covers the whole file, including the bytes from attempt one.
    assert!(done.verified);
    assert_eq!(done.bytes, body.len() as u64);
    assert_eq!(std::fs::read(&done.path).unwrap(), body);
    assert!(!part.exists(), "the .part file was not cleaned up");
}

#[tokio::test]
async fn a_server_that_ignores_range_is_not_appended_to() {
    // A server that answers 200 to a Range request is sending the whole file.
    // Appending that to the partial bytes would silently produce a broken APK
    // of the wrong length, so the download has to start over instead.
    let body = payload(300 * 1024);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = body.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let body = served.clone();
            std::thread::spawn(move || {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let _ = reader.read_line(&mut line);
                loop {
                    let mut header = String::new();
                    match reader.read_line(&mut header) {
                        Ok(0) => break,
                        Ok(_) if header.trim().is_empty() => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                // Always 200, always the whole body, whatever was asked for.
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("com.example.app_1.0_7_arm64-v8a.apk.part");
    std::fs::write(&part, &body[..1024]).unwrap();

    let url = format!("http://127.0.0.1:{port}/app.apk");
    let variant = variant(&url, body.len() as u64, Some(sha1_hex(&body)));
    let done = Client::new()
        .download(&variant, dir.path(), &CancelToken::new(), |_| {})
        .await
        .expect("it should start over rather than corrupt the file");

    assert_eq!(done.bytes, body.len() as u64);
    assert!(done.verified);
    assert_eq!(std::fs::read(&done.path).unwrap(), body);
}

#[tokio::test]
async fn a_rate_limited_download_says_so() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut sink = String::new();
            let _ = reader.read_line(&mut sink);
            let _ = stream.write_all(
                b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 1800\r\nContent-Length: 0\r\n\r\n",
            );
        }
    });

    let url = format!("http://127.0.0.1:{port}/app.apk");
    let dir = tempfile::tempdir().unwrap();
    let error = Client::new()
        .download(
            &variant(&url, 1, None),
            dir.path(),
            &CancelToken::new(),
            |_| {},
        )
        .await
        .expect_err("429 must not look like success");

    match error {
        // Capped, so a hostile value cannot park the UI for half an hour.
        Error::RateLimited { retry_after_secs } => assert_eq!(retry_after_secs, 60),
        other => panic!("expected a rate-limit error, got {other:?}"),
    }
}
