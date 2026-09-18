//! Reading XAPK bundles.
//!
//! A bundle is a zip of split APKs. The APKs here are assembled around the real
//! manifest fixture, so the metadata read back is known-correct without needing
//! a multi-hundred-megabyte file in the repository.

use std::io::Write;
use std::path::Path;

use xylitol_apk::{inspect, PackageKind};

const MANIFEST: &[u8] = include_bytes!("fixtures/fdroid-AndroidManifest.xml");

/// A minimal but genuine APK: a zip with an `AndroidManifest.xml` in it.
fn write_apk(path: &Path, extra: &[(&str, &[u8])]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("AndroidManifest.xml", options).unwrap();
    zip.write_all(MANIFEST).unwrap();
    for (name, bytes) in extra {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn write_bundle(path: &Path, members: &[(&str, &Path)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, source) in members {
        zip.start_file(*name, options).unwrap();
        zip.write_all(&std::fs::read(source).unwrap()).unwrap();
    }
    zip.start_file("manifest.json", options).unwrap();
    zip.write_all(br#"{"package_name":"org.fdroid.fdroid"}"#)
        .unwrap();
    zip.finish().unwrap();
}

#[test]
fn a_plain_apk_reports_its_abis_from_its_lib_directory() {
    let dir = tempfile::tempdir().unwrap();
    let apk = dir.path().join("app.apk");
    write_apk(
        &apk,
        &[
            ("lib/arm64-v8a/libfoo.so", b"\x7fELF"),
            ("lib/armeabi-v7a/libfoo.so", b"\x7fELF"),
            ("res/drawable/icon.png", b"\x89PNG"),
        ],
    );

    let info = inspect(&apk).unwrap();
    assert_eq!(info.kind, PackageKind::Apk);
    assert_eq!(info.abis, vec!["arm64-v8a", "armeabi-v7a"]);
    assert_eq!(info.package, "org.fdroid.fdroid");
    assert!(info.file_size > 0);
    assert_eq!(info.sha256.len(), 64);
}

#[test]
fn a_bundle_is_described_by_its_base_apk() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base.apk");
    let split = dir.path().join("split.apk");
    write_apk(&base, &[]);
    write_apk(&split, &[]);

    let bundle = dir.path().join("app.xapk");
    write_bundle(
        &bundle,
        &[
            // Deliberately listed split-first: the base must be found by shape,
            // not by position.
            ("config.arm64_v8a.apk", split.as_path()),
            ("org.fdroid.fdroid.apk", base.as_path()),
        ],
    );

    let info = inspect(&bundle).unwrap();
    assert_eq!(info.kind, PackageKind::Bundle);
    assert_eq!(info.package, "org.fdroid.fdroid");
    assert_eq!(info.version_code, Some(1_021_051));
    assert_eq!(info.contained_apks.len(), 2);
    // The ABI is implied by the split's name.
    assert_eq!(info.abis, vec!["arm64-v8a"]);
}

#[test]
fn a_zip_with_no_package_in_it_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.zip");
    let file = std::fs::File::create(&path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
    zip.start_file("readme.txt", options).unwrap();
    zip.write_all(b"nothing to see").unwrap();
    zip.finish().unwrap();

    assert!(matches!(inspect(&path), Err(xylitol_apk::Error::NotAnApk)));
}

#[test]
fn a_file_that_is_not_a_zip_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, b"\xff\xd8\xff\xe0 not a zip").unwrap();
    assert!(matches!(inspect(&path), Err(xylitol_apk::Error::Zip(_))));
}
