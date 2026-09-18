//! Parser tests against real APKPure markup.
//!
//! The fixtures are trimmed excerpts of pages captured from the live site. They
//! exist so that a change in APKPure's markup shows up as a failing test rather
//! than as an empty list in the UI.

use xylitol_apkpure::parse;
use xylitol_apkpure::FileKind;

const PACKAGE: &str = "org.videolan.vlc";

#[test]
fn reads_every_release_from_a_versions_page() {
    let html = include_str!("fixtures/versions.html");
    let releases = parse::releases(html, PACKAGE).expect("versions page should parse");

    assert!(
        releases.len() >= 30,
        "expected the full version history, got {}",
        releases.len()
    );

    let latest = &releases[0];
    assert_eq!(latest.package, PACKAGE);
    assert_eq!(latest.version_name, "3.7.1");
    assert_eq!(latest.version_code, Some(13070106));
    assert_eq!(latest.kinds, vec![FileKind::Apk]);
    assert_eq!(latest.size, Some(47_990_842));
    assert!(latest.has_variants, "3.7.1 ships more than one file");

    // Both file formats appear in this app's history, and at least one release
    // is published in both at once — the case a single `kind` field got wrong.
    assert!(releases.iter().any(|r| r.offers(FileKind::Xapk)));
    assert!(releases.iter().any(|r| r.offers(FileKind::Apk)));
    let both = releases
        .iter()
        .find(|r| r.offers(FileKind::Apk) && r.offers(FileKind::Xapk))
        .expect("3.7.0 is published as both an APK and an XAPK");
    assert_eq!(both.kinds_display(), "APK + XAPK");

    // Every release must be actionable: without a version code there is no
    // download URL to build.
    let missing: Vec<&str> = releases
        .iter()
        .filter(|r| r.version_code.is_none())
        .map(|r| r.version_name.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "releases without a version code: {missing:?}"
    );
}

#[test]
fn reads_every_variant_from_a_download_page() {
    let html = include_str!("fixtures/download.html");
    let variants = parse::variants(html, PACKAGE).expect("download page should parse");

    assert!(
        variants.len() >= 2,
        "expected several per-ABI builds, got {}",
        variants.len()
    );

    for variant in &variants {
        assert_eq!(variant.package, PACKAGE);
        assert!(
            variant.download_url.starts_with("https://d.apkpure.com/b/"),
            "unexpected download URL: {}",
            variant.download_url
        );
        assert!(
            variant
                .download_url
                .contains(&variant.version_code.to_string()),
            "download URL does not name the build it is for: {}",
            variant.download_url
        );
        assert!(!variant.version_name.is_empty());
    }

    // The ABI is what distinguishes these files, and it is what the picker
    // filters on, so it has to survive parsing.
    let arm64 = variants
        .iter()
        .find(|v| v.arch.as_deref() == Some("arm64-v8a"))
        .expect("an arm64-v8a build should be listed");
    assert_eq!(arm64.dpi.as_deref(), Some("nodpi"));
    assert!(arm64
        .min_android
        .as_deref()
        .unwrap_or("")
        .contains("Android"));
    assert_eq!(
        arm64.sha1.as_ref().map(String::len),
        Some(40),
        "a published SHA-1 is what lets a download be verified"
    );

    let v7a = variants
        .iter()
        .find(|v| v.arch.as_deref() == Some("armeabi-v7a"))
        .expect("an armeabi-v7a build should be listed");
    assert_ne!(
        arm64.version_code, v7a.version_code,
        "per-ABI builds have distinct version codes"
    );

    // Descriptors are what the user reads when choosing; they must differ.
    let descriptors: std::collections::BTreeSet<String> =
        variants.iter().map(|v| v.descriptor()).collect();
    assert!(
        descriptors.len() > 1,
        "every variant described itself identically: {descriptors:?}"
    );
}

#[test]
fn suggested_filenames_are_unique_per_variant() {
    let html = include_str!("fixtures/download.html");
    let variants = parse::variants(html, PACKAGE).unwrap();
    let names: std::collections::BTreeSet<String> =
        variants.iter().map(|v| v.suggested_filename()).collect();
    assert_eq!(
        names.len(),
        variants.len(),
        "two variants would be saved over each other"
    );
}
