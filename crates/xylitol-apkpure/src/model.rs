//! Types describing what APKPure publishes for an app.

use serde::{Deserialize, Serialize};

/// Whether a file is a single APK or an XAPK/APKS bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileKind {
    Apk,
    Xapk,
}

impl FileKind {
    /// The path segment APKPure uses for this kind.
    pub fn as_path_segment(self) -> &'static str {
        match self {
            FileKind::Apk => "APK",
            FileKind::Xapk => "XAPK",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            FileKind::Apk => "apk",
            FileKind::Xapk => "xapk",
        }
    }

    pub fn parse(s: &str) -> Option<FileKind> {
        match s.trim().to_ascii_uppercase().as_str() {
            "APK" => Some(FileKind::Apk),
            "XAPK" | "APKS" => Some(FileKind::Xapk),
            _ => None,
        }
    }
}

impl std::fmt::Display for FileKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_path_segment())
    }
}

/// One app returned by a search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub package: String,
    pub title: String,
    pub icon_url: Option<String>,
    pub developer: Option<String>,
    pub installs: Option<String>,
    pub score: Option<String>,
    pub latest_version: Option<String>,
    pub latest_version_code: Option<i64>,
    pub latest_size: Option<u64>,
}

/// One published release, as listed on the app's versions page.
///
/// A release can have several downloadable files — per-ABI or per-DPI builds,
/// and an APK and an XAPK of the same version side by side. Those are
/// [`Variant`]s and are fetched separately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub package: String,
    pub version_name: String,
    pub version_code: Option<i64>,
    /// Every file format this release is published in, in page order.
    pub kinds: Vec<FileKind>,
    pub size: Option<u64>,
    pub published: Option<String>,
    /// True when this release ships more than one file to choose between.
    pub has_variants: bool,
}

impl Release {
    /// `APK`, or `APK + XAPK` when both are offered.
    pub fn kinds_display(&self) -> String {
        if self.kinds.is_empty() {
            return FileKind::Apk.to_string();
        }
        self.kinds
            .iter()
            .map(FileKind::to_string)
            .collect::<Vec<_>>()
            .join(" + ")
    }

    pub fn offers(&self, kind: FileKind) -> bool {
        self.kinds.contains(&kind)
    }
}

/// A single downloadable file: the thing the user actually picks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variant {
    pub package: String,
    pub version_name: String,
    pub version_code: i64,
    pub kind: FileKind,
    pub size: Option<u64>,
    pub published: Option<String>,
    /// `armeabi-v7a`, `arm64-v8a`, `universal`, ...
    pub arch: Option<String>,
    pub dpi: Option<String>,
    /// Minimum Android version, as APKPure renders it (`Android 4.2+`).
    pub min_android: Option<String>,
    /// SHA-1 of the file, when APKPure publishes it. Used to verify downloads.
    pub sha1: Option<String>,
    /// SHA-1 of the signing certificate.
    pub signature: Option<String>,
    pub uploader: Option<String>,
    pub download_url: String,
}

impl Variant {
    /// A stable, human-readable description of what makes this file different
    /// from its siblings — what the picker shows in each row.
    pub fn descriptor(&self) -> String {
        let mut parts = Vec::new();
        parts.push(self.arch.clone().unwrap_or_else(|| "universal".into()));
        if let Some(dpi) = &self.dpi {
            if dpi != "nodpi" {
                parts.push(dpi.clone());
            }
        }
        parts.push(self.kind.to_string());
        parts.join(" · ")
    }

    /// The filename Xylitol saves this variant as.
    pub fn suggested_filename(&self) -> String {
        let arch = self
            .arch
            .as_deref()
            .unwrap_or("universal")
            .replace(['/', ' '], "-");
        format!(
            "{}_{}_{}_{}.{}",
            self.package,
            self.version_name.replace([' ', '/'], "-"),
            self.version_code,
            arch,
            self.kind.extension()
        )
    }
}
