//! A client for APKPure.
//!
//! APKPure exposes a JSON endpoint for search but nothing official for version
//! history, so releases and variants are read from the website's markup (see
//! [`parse`]). Downloads go through a stable, unsigned URL — APKPure redirects
//! it to a short-lived signed CDN link — which means Xylitol can record a
//! variant now and fetch it later.

pub mod model;
pub mod parse;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use sha1::{Digest, Sha1};
use tokio::io::AsyncWriteExt;

pub use model::{FileKind, Release, SearchHit, Variant};

/// Presenting as a desktop browser is required: APKPure's edge answers 403, or
/// redirects to the front page, for clients it does not recognise.
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";

/// How many times a rate-limited request is retried before giving up.
const MAX_RETRIES: u32 = 3;

const SEARCH_ENDPOINT: &str = "https://apkpure.com/api/v1/search_suggestion_new";
const SITE: &str = "https://apkpure.com";
const CDN: &str = "https://d.apkpure.com";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("APKPure returned {status} for {url}")]
    Status { status: u16, url: String },
    #[error("APKPure is rate limiting this address; try again in about {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("APKPure has no app with the package name `{0}`")]
    NotFound(String),
    #[error("no downloadable file matched that selection")]
    NoSuchVariant,
    #[error("the downloaded file is corrupt: expected SHA-1 {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("download was cancelled")]
    Cancelled,
    #[error("could not read APKPure's response: {0}")]
    Parse(String),
}

/// Progress of an in-flight download.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub downloaded: u64,
    /// Total size, when the server reported one.
    pub total: Option<u64>,
}

impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(t) if t > 0 => Some((self.downloaded as f64 / t as f64).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

/// A finished download.
#[derive(Debug, Clone)]
pub struct Downloaded {
    pub path: PathBuf,
    pub bytes: u64,
    pub sha1: String,
    /// `true` when APKPure published a checksum and it matched.
    pub verified: bool,
}

/// Cooperative cancellation for a download in progress.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<std::sync::atomic::AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(30))
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(browser_headers())
            .connect_timeout(timeout)
            // No overall timeout: downloads of large bundles legitimately run
            // for minutes. Stalls are caught by the per-read timeout instead.
            .read_timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .expect("reqwest client with static config must build");
        Client { http }
    }

    /// Search APKPure by app name or package name.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, Error> {
        let url = format!(
            "{SEARCH_ENDPOINT}?key={}&limit={}",
            urlencoding::encode(query.trim()),
            limit.clamp(1, 50)
        );
        let body: serde_json::Value = self
            .get(&url)
            .await?
            .json()
            .await
            .map_err(|e| Error::Parse(e.to_string()))?;

        let items = body
            .as_array()
            .ok_or_else(|| Error::Parse("search response was not a list".into()))?;

        Ok(items
            .iter()
            .filter_map(|it| {
                let package = it.get("packageName")?.as_str()?.to_string();
                Some(SearchHit {
                    title: it
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&package)
                        .to_string(),
                    package,
                    icon_url: it.get("icon").and_then(|v| v.as_str()).map(str::to_string),
                    developer: it
                        .get("developer")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    installs: it
                        .get("installTotal")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    score: it.get("score").and_then(|v| v.as_str()).map(str::to_string),
                    latest_version: it
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    latest_version_code: it.get("versionCode").and_then(|v| v.as_i64()),
                    latest_size: it.get("fileSize").and_then(|v| v.as_u64()),
                })
            })
            .collect())
    }

    /// Every published release of an app, newest first.
    pub async fn releases(&self, package: &str) -> Result<Vec<Release>, Error> {
        // The slug in the path is cosmetic: APKPure redirects any slug to the
        // canonical one as long as the package name is right.
        let url = format!("{SITE}/a/{package}/versions");
        let html = self.get_text(&url).await?;
        parse::releases(&html, package)
    }

    /// Every downloadable file for an app, or for one specific release.
    ///
    /// This is the list the user picks from: one entry per ABI/DPI/format
    /// combination that APKPure hosts.
    pub async fn variants(
        &self,
        package: &str,
        version_name: Option<&str>,
    ) -> Result<Vec<Variant>, Error> {
        let url = match version_name {
            Some(v) => format!("{SITE}/a/{package}/download/{}", urlencoding::encode(v)),
            None => format!("{SITE}/a/{package}/download"),
        };
        let html = self.get_text(&url).await?;
        let mut variants = parse::variants(&html, package)?;
        // Newest build first, then widest ABI first, so the safest pick is on top.
        variants.sort_by(|a, b| {
            b.version_code
                .cmp(&a.version_code)
                .then_with(|| arch_rank(&a.arch).cmp(&arch_rank(&b.arch)))
        });
        Ok(variants)
    }

    /// The stable download URL for a specific build.
    ///
    /// Useful when the version code is already known and the variant list does
    /// not need to be fetched.
    pub fn download_url(package: &str, kind: FileKind, version_code: i64) -> String {
        format!(
            "{CDN}/b/{}/{package}?versionCode={version_code}",
            kind.as_path_segment()
        )
    }

    /// Download a variant into `dir`, resuming a previous partial download when
    /// one is present.
    ///
    /// `on_progress` is called as bytes arrive. The file is written to a
    /// `.part` sibling and renamed only once complete, so an interrupted run can
    /// never leave a truncated APK that looks installable.
    pub async fn download(
        &self,
        variant: &Variant,
        dir: impl AsRef<Path>,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(Progress),
    ) -> Result<Downloaded, Error> {
        let dir = dir.as_ref();
        tokio::fs::create_dir_all(dir).await?;
        let final_path = dir.join(variant.suggested_filename());
        let part_path = final_path.with_extension(format!("{}.part", variant.kind.extension()));

        let already = tokio::fs::metadata(&part_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);

        let mut request = self
            .http
            .get(&variant.download_url)
            .header(reqwest::header::REFERER, SITE);
        if already > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={already}-"));
        }

        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(Error::RateLimited {
                retry_after_secs: retry_after(&response).unwrap_or(backoff(1)).as_secs(),
            });
        }
        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                url: variant.download_url.clone(),
            });
        }

        // If the server ignored our Range request we must start over, otherwise
        // the resumed bytes would be prepended to a full copy of the file.
        let resuming = already > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        let start_at = if resuming { already } else { 0 };

        let total = response
            .content_length()
            .map(|len| len + start_at)
            .or(variant.size);

        let mut hasher = Sha1::new();
        let mut file = if resuming {
            // Re-hash what is already on disk so the final checksum covers it.
            let existing = tokio::fs::read(&part_path).await?;
            hasher.update(&existing);
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part_path)
                .await?
        } else {
            tokio::fs::File::create(&part_path).await?
        };

        let mut downloaded = start_at;
        on_progress(Progress { downloaded, total });

        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if cancel.is_cancelled() {
                // Leave the .part file: the next attempt resumes from here.
                file.flush().await?;
                return Err(Error::Cancelled);
            }
            let chunk = chunk?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            on_progress(Progress { downloaded, total });
        }
        file.flush().await?;
        drop(file);

        let sha1 = hex(&hasher.finalize());
        let verified = match &variant.sha1 {
            Some(expected) if !expected.is_empty() => {
                if expected.eq_ignore_ascii_case(&sha1) {
                    true
                } else {
                    // A bad file is worse than no file: do not keep it around to
                    // be resumed into permanent corruption.
                    let _ = tokio::fs::remove_file(&part_path).await;
                    return Err(Error::ChecksumMismatch {
                        expected: expected.clone(),
                        actual: sha1,
                    });
                }
            }
            _ => false,
        };

        tokio::fs::rename(&part_path, &final_path).await?;
        Ok(Downloaded {
            path: final_path,
            bytes: downloaded,
            sha1,
            verified,
        })
    }

    /// GET with a retry for 429, which APKPure applies per source address.
    ///
    /// A desktop client browsing an app's history makes a burst of requests, so
    /// backing off politely is the difference between working and not.
    async fn get(&self, url: &str) -> Result<reqwest::Response, Error> {
        let mut attempt = 0;
        loop {
            let response = self
                .http
                .get(url)
                .header(reqwest::header::REFERER, SITE)
                .send()
                .await?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = retry_after(&response).unwrap_or_else(|| backoff(attempt));
                if attempt >= MAX_RETRIES {
                    return Err(Error::RateLimited {
                        retry_after_secs: wait.as_secs(),
                    });
                }
                tracing::warn!(url, attempt, "rate limited, waiting {:?}", wait);
                tokio::time::sleep(wait).await;
                attempt += 1;
                continue;
            }

            return Err(Error::Status {
                status: status.as_u16(),
                url: url.to_string(),
            });
        }
    }

    async fn get_text(&self, url: &str) -> Result<String, Error> {
        Ok(self.get(url).await?.text().await?)
    }
}

/// The header set a current Chrome sends. Cloudflare scores requests on these,
/// and a bare `User-Agent` alone is not enough to be served.
fn browser_headers() -> reqwest::header::HeaderMap {
    use reqwest::header::{HeaderMap, HeaderValue};
    let mut headers = HeaderMap::new();
    let pairs = [
        (
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
        ),
        ("accept-language", "en-US,en;q=0.9"),
        (
            "sec-ch-ua",
            "\"Chromium\";v=\"140\", \"Not;A=Brand\";v=\"24\"",
        ),
        ("sec-ch-ua-mobile", "?0"),
        ("sec-ch-ua-platform", "\"Linux\""),
        ("sec-fetch-dest", "document"),
        ("sec-fetch-mode", "navigate"),
        ("sec-fetch-site", "none"),
        ("sec-fetch-user", "?1"),
        ("upgrade-insecure-requests", "1"),
    ];
    for (name, value) in pairs {
        headers.insert(name, HeaderValue::from_static(value));
    }
    headers
}

/// Honour a `Retry-After` header when the server sends one.
fn retry_after(response: &reqwest::Response) -> Option<Duration> {
    let value = response.headers().get(reqwest::header::RETRY_AFTER)?;
    let secs: u64 = value.to_str().ok()?.trim().parse().ok()?;
    // Cap it: a hostile or buggy value should not hang the UI for an hour.
    Some(Duration::from_secs(secs.min(60)))
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.pow(attempt.min(4)))
}

/// Order ABIs so that the most broadly compatible build sorts first.
fn arch_rank(arch: &Option<String>) -> u8 {
    match arch.as_deref().map(str::trim) {
        None | Some("") => 0,
        Some(a) if a.eq_ignore_ascii_case("universal") => 0,
        Some(a) if a.contains(',') => 1,
        Some("arm64-v8a") => 2,
        Some("armeabi-v7a") => 3,
        Some("x86_64") => 4,
        Some("x86") => 5,
        _ => 6,
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_urls_are_stable() {
        assert_eq!(
            Client::download_url("org.videolan.vlc", FileKind::Apk, 13070105),
            "https://d.apkpure.com/b/APK/org.videolan.vlc?versionCode=13070105"
        );
        assert_eq!(
            Client::download_url("com.example", FileKind::Xapk, 7),
            "https://d.apkpure.com/b/XAPK/com.example?versionCode=7"
        );
    }

    #[test]
    fn universal_builds_sort_before_abi_specific_ones() {
        let mut ranks = [
            arch_rank(&Some("x86".into())),
            arch_rank(&None),
            arch_rank(&Some("arm64-v8a".into())),
        ];
        ranks.sort();
        assert_eq!(ranks, [0, 2, 5]);
    }

    #[test]
    fn filenames_encode_the_choice() {
        let v = Variant {
            package: "com.example".into(),
            version_name: "1.0 Beta 2".into(),
            version_code: 12,
            kind: FileKind::Apk,
            size: None,
            published: None,
            arch: Some("arm64-v8a".into()),
            dpi: None,
            min_android: None,
            sha1: None,
            signature: None,
            uploader: None,
            download_url: String::new(),
        };
        assert_eq!(
            v.suggested_filename(),
            "com.example_1.0-Beta-2_12_arm64-v8a.apk"
        );
    }
}
