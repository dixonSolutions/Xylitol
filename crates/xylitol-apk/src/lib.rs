//! Reading Android packages.
//!
//! Xylitol needs to know what a downloaded file actually contains before it
//! offers to install it: the package name, version, ABIs and the permissions it
//! will ask for. This crate answers those questions from the file alone, with
//! no Android tooling installed.

pub mod axml;

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use axml::{Element, Value};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("this file has no AndroidManifest.xml, so it is not an APK")]
    NotAnApk,
    #[error("malformed AndroidManifest.xml: {0}")]
    Axml(String),
}

/// Which flavour of package a file turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKind {
    /// A single installable APK.
    Apk,
    /// A split APK: installable only together with its base.
    SplitApk,
    /// An XAPK/APKS bundle: a zip holding a base APK plus splits and assets.
    Bundle,
}

/// Everything Xylitol reads out of a package file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub kind: PackageKind,
    pub package: String,
    pub version_name: Option<String>,
    pub version_code: Option<i64>,
    pub label: Option<String>,
    pub min_sdk: Option<i64>,
    pub target_sdk: Option<i64>,
    /// `config.arm64_v8a`-style split name, when this is a split APK.
    pub split: Option<String>,
    /// ABIs with native code in `lib/<abi>/`.
    pub abis: Vec<String>,
    pub permissions: Vec<String>,
    /// Activities declared with a LAUNCHER intent filter.
    pub launchable_activities: Vec<String>,
    /// For bundles: the APKs found inside.
    pub contained_apks: Vec<String>,
    pub file_size: u64,
    pub sha256: String,
}

impl PackageInfo {
    /// A short name suitable for a list row.
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.package)
    }

    /// `1.2.3 (45)` when both parts are known.
    pub fn version_display(&self) -> String {
        match (&self.version_name, self.version_code) {
            (Some(n), Some(c)) => format!("{n} ({c})"),
            (Some(n), None) => n.clone(),
            (None, Some(c)) => c.to_string(),
            (None, None) => "unknown".into(),
        }
    }
}

/// Read a package from disk.
///
/// Handles plain APKs and XAPK/APKS bundles; for a bundle the metadata comes
/// from the base APK inside it.
pub fn inspect(path: impl AsRef<Path>) -> Result<PackageInfo, Error> {
    let path = path.as_ref();
    let file_size = std::fs::metadata(path)?.len();
    let sha256 = sha256_file(path)?;

    let mut zip = zip::ZipArchive::new(File::open(path)?)?;

    if let Some(manifest) = read_entry(&mut zip, "AndroidManifest.xml")? {
        let mut info = from_manifest(&manifest)?;
        info.abis = abis_in(&zip);
        info.file_size = file_size;
        info.sha256 = sha256;
        return Ok(info);
    }

    // No manifest at the top level: treat it as a bundle and look inside.
    let inner_apks: Vec<String> = zip
        .file_names()
        .filter(|n| n.to_ascii_lowercase().ends_with(".apk"))
        .map(str::to_string)
        .collect();
    if inner_apks.is_empty() {
        return Err(Error::NotAnApk);
    }

    // The base APK is the one without a split name; fall back to the first.
    let mut sorted = inner_apks.clone();
    sorted.sort_by_key(|n| (n.contains("config."), n.contains("split"), n.len()));
    let base_name = sorted[0].clone();

    let base_bytes = read_entry(&mut zip, &base_name)?.ok_or(Error::NotAnApk)?;
    let mut base_zip = zip::ZipArchive::new(std::io::Cursor::new(base_bytes))?;
    let manifest = read_entry(&mut base_zip, "AndroidManifest.xml")?.ok_or(Error::NotAnApk)?;

    let mut info = from_manifest(&manifest)?;
    info.kind = PackageKind::Bundle;
    info.abis = inner_apks
        .iter()
        .filter_map(|n| {
            n.rsplit_once("config.")
                .map(|(_, abi)| abi.trim_end_matches(".apk"))
        })
        .filter(|abi| abi.contains("arm") || abi.contains("x86") || abi.contains("mips"))
        .map(|abi| abi.replace('_', "-"))
        .collect();
    info.contained_apks = inner_apks;
    info.file_size = file_size;
    info.sha256 = sha256;
    Ok(info)
}

/// Read a binary `AndroidManifest.xml` on its own.
///
/// The file-level fields (`file_size`, `sha256`, `abis`) are left empty, since
/// a manifest carries none of them. [`inspect`] fills them in.
pub fn parse_manifest(manifest: &[u8]) -> Result<PackageInfo, Error> {
    from_manifest(manifest)
}

fn from_manifest(manifest: &[u8]) -> Result<PackageInfo, Error> {
    let root = axml::parse(manifest)?;

    let package = root.attr_str("package").unwrap_or_default().to_string();
    let split = root.attr_str("split").map(str::to_string);

    // `versionCode` may be stored as a 32-bit int; `versionCodeMajor` extends it.
    let version_code = root.attr_i64("versionCode");
    let version_name = root.attr("versionName").map(|v| v.to_display());

    let uses_sdk = root.child("uses-sdk");
    let min_sdk = uses_sdk.and_then(|e| e.attr_i64("minSdkVersion"));
    let target_sdk = uses_sdk.and_then(|e| e.attr_i64("targetSdkVersion"));

    let permissions: Vec<String> = root
        .children_named("uses-permission")
        .chain(root.children_named("uses-permission-sdk-23"))
        .filter_map(|e| e.attr_str("name"))
        .map(str::to_string)
        .collect();

    let application = root.child("application");
    // The label is usually a resource reference; only a literal is useful here.
    let label = application
        .and_then(|a| a.attr("label"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut launchable = Vec::new();
    if let Some(app) = application {
        for activity in app
            .children_named("activity")
            .chain(app.children_named("activity-alias"))
        {
            let is_launcher = activity.children_named("intent-filter").any(|f| {
                f.children_named("category")
                    .any(|c| c.attr_str("name") == Some("android.intent.category.LAUNCHER"))
            });
            if is_launcher {
                if let Some(name) = activity.attr_str("name") {
                    launchable.push(name.to_string());
                }
            }
        }
    }

    let kind = if split.is_some() {
        PackageKind::SplitApk
    } else {
        PackageKind::Apk
    };

    Ok(PackageInfo {
        kind,
        package,
        version_name,
        version_code,
        label,
        min_sdk,
        target_sdk,
        split,
        abis: Vec::new(),
        permissions,
        launchable_activities: launchable,
        contained_apks: Vec::new(),
        file_size: 0,
        sha256: String::new(),
    })
}

/// ABIs are implied by the `lib/<abi>/` directory layout.
fn abis_in<R: Read + Seek>(zip: &zip::ZipArchive<R>) -> Vec<String> {
    let mut set = BTreeSet::new();
    for name in zip.file_names() {
        if let Some(rest) = name.strip_prefix("lib/") {
            if let Some((abi, _)) = rest.split_once('/') {
                set.insert(abi.to_string());
            }
        }
    }
    set.into_iter().collect()
}

fn read_entry<R: Read + Seek>(
    zip: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>, Error> {
    match zip.by_name(name) {
        Ok(mut f) => {
            let mut buf = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut buf)?;
            Ok(Some(buf))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn sha256_file(path: &Path) -> Result<String, Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}
