//! The binary XML decoder, checked against a manifest from a real APK.
//!
//! The fixture is `AndroidManifest.xml` taken verbatim from F-Droid 1.21.1
//! (`org.fdroid.fdroid_1021051.apk`). It is a good specimen: a large manifest
//! with namespaced attributes, an `activity-alias`, dozens of permissions and
//! both framework and app-defined names.

use xylitol_apk::{parse_manifest, PackageKind};

const MANIFEST: &[u8] = include_bytes!("fixtures/fdroid-AndroidManifest.xml");

#[test]
fn reads_the_identity_of_a_real_package() {
    let info = parse_manifest(MANIFEST).expect("a real manifest should decode");

    assert_eq!(info.package, "org.fdroid.fdroid");
    assert_eq!(info.version_name.as_deref(), Some("1.21.1"));
    assert_eq!(info.version_code, Some(1_021_051));
    assert_eq!(info.kind, PackageKind::Apk);
    assert_eq!(info.split, None);
    assert_eq!(info.version_display(), "1.21.1 (1021051)");
}

#[test]
fn reads_the_sdk_range() {
    let info = parse_manifest(MANIFEST).unwrap();
    assert_eq!(info.min_sdk, Some(23));
    assert_eq!(info.target_sdk, Some(29));
}

#[test]
fn reads_every_requested_permission() {
    let info = parse_manifest(MANIFEST).unwrap();

    assert_eq!(info.permissions.len(), 29, "got: {:#?}", info.permissions);
    // Framework permissions and the app's own must both survive.
    assert!(info
        .permissions
        .contains(&"android.permission.INTERNET".to_string()));
    assert!(info
        .permissions
        .contains(&"android.permission.CAMERA".to_string()));
    assert!(info
        .permissions
        .contains(&"org.fdroid.fdroid.permission.UPDATE_REPOS".to_string()));
    // Every entry must be a real name, not a leaked resource reference.
    for permission in &info.permissions {
        assert!(!permission.is_empty());
        assert!(
            !permission.starts_with('@'),
            "unresolved value: {permission}"
        );
    }
}

#[test]
fn finds_launcher_activities_including_aliases() {
    let info = parse_manifest(MANIFEST).unwrap();

    assert!(
        info.launchable_activities
            .contains(&"org.fdroid.fdroid.views.main.MainActivity".to_string()),
        "got: {:?}",
        info.launchable_activities
    );
    // F-Droid's panic mode disguises the app behind a calculator, declared as an
    // <activity-alias>: proof that aliases are walked, not just activities.
    assert!(info
        .launchable_activities
        .contains(&"org.fdroid.fdroid.panic.CalculatorActivity".to_string()));
}

#[test]
fn a_truncated_manifest_is_an_error_not_a_panic() {
    for cut in [0, 4, 8, 64, MANIFEST.len() / 2, MANIFEST.len() - 1] {
        let result = parse_manifest(&MANIFEST[..cut]);
        assert!(
            result.is_err() || result.unwrap().package == "org.fdroid.fdroid",
            "truncating to {cut} bytes should fail cleanly"
        );
    }
}

#[test]
fn arbitrary_bytes_are_an_error_not_a_panic() {
    assert!(parse_manifest(b"").is_err());
    assert!(parse_manifest(b"not xml at all").is_err());
    assert!(parse_manifest(&[0xff; 512]).is_err());
}
