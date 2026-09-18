//! End-to-end tests against the real APKPure.
//!
//! Ignored by default. APKPure rate-limits by source address and answers 429
//! with a `Retry-After` measured in tens of minutes, so these run on request:
//!
//! ```sh
//! cargo test -p xylitol-apkpure -- --ignored --test-threads=1
//! ```

use xylitol_apkpure::{CancelToken, Client, FileKind};

/// A small, stable, freely redistributable app to exercise the download path.
const SMALL_APP: &str = "org.videolan.vlc";

#[tokio::test]
#[ignore = "hits the live site; APKPure rate-limits by address"]
async fn search_finds_an_app_by_name() {
    let hits = Client::new().search("vlc", 5).await.expect("search");
    assert!(
        !hits.is_empty(),
        "no results for a query that certainly matches"
    );
    assert!(
        hits.iter().any(|h| h.package == SMALL_APP),
        "expected {SMALL_APP} among: {:?}",
        hits.iter().map(|h| &h.package).collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "hits the live site; APKPure rate-limits by address"]
async fn release_history_is_readable() {
    let releases = Client::new().releases(SMALL_APP).await.expect("releases");
    assert!(releases.len() > 5, "expected a version history");
    assert!(releases.iter().all(|r| r.version_code.is_some()));
}

#[tokio::test]
#[ignore = "hits the live site; APKPure rate-limits by address"]
async fn variants_carry_everything_needed_to_choose() {
    let variants = Client::new()
        .variants(SMALL_APP, None)
        .await
        .expect("variants");
    assert!(!variants.is_empty());
    for v in &variants {
        assert!(v.download_url.starts_with("https://d.apkpure.com/b/"));
        assert_eq!(v.package, SMALL_APP);
    }
}

#[tokio::test]
#[ignore = "hits the live site; downloads tens of megabytes"]
async fn a_download_is_a_real_verified_package() {
    let client = Client::new();
    let variants = client.variants(SMALL_APP, None).await.expect("variants");

    // Prefer a build with a published checksum: it proves the verify path.
    let variant = variants
        .iter()
        .find(|v| v.sha1.is_some() && v.kind == FileKind::Apk)
        .or_else(|| variants.first())
        .expect("at least one file");

    let dir = tempfile::tempdir().expect("temp dir");
    let mut seen_progress = false;
    let done = client
        .download(variant, dir.path(), &CancelToken::new(), |_| {
            seen_progress = true;
        })
        .await
        .expect("download");

    assert!(seen_progress, "progress was never reported");
    assert!(done.bytes > 1_000_000, "suspiciously small file");
    if variant.sha1.is_some() {
        assert!(
            done.verified,
            "a published checksum should have been checked"
        );
    }

    // The bytes must be a package we can actually read back.
    let info = xylitol_apk::inspect(&done.path).expect("inspect the downloaded file");
    assert_eq!(info.package, SMALL_APP);
    assert_eq!(info.version_code, Some(variant.version_code));
    assert!(!info.permissions.is_empty());
}
