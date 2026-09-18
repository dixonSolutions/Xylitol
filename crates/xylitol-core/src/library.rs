//! The local library: what has been downloaded, and what it is.
//!
//! The index is a plain JSON file so that it can be inspected or deleted by
//! hand. Entries whose file has disappeared are dropped on load rather than
//! shown as broken rows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use xylitol_apk::PackageInfo;
use xylitol_apkpure::Variant;

use crate::paths;

/// One downloaded file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: PathBuf,
    pub info: PackageInfo,
    /// The APKPure variant this came from, when it was not added from disk.
    pub source: Option<Variant>,
    /// RFC 3339 timestamp.
    pub added: String,
    /// Whether the download matched APKPure's published checksum.
    pub verified: bool,
}

impl Entry {
    /// A stable identifier, as shown by `library list` and taken by `remove`.
    pub fn key(&self) -> String {
        format!(
            "{}@{}#{}",
            self.info.package,
            self.info.version_code.unwrap_or(-1),
            self.discriminator()
        )
    }

    /// What distinguishes this file from others of the same package and version.
    ///
    /// Publishers do not always give per-ABI builds distinct version codes, and
    /// an APK and a bundle of one version can both exist, so package and version
    /// alone are not unique — keying on those alone let one download silently
    /// replace another in the index while both files sat on disk.
    fn discriminator(&self) -> String {
        if let Some(split) = &self.info.split {
            return split.clone();
        }
        if self.info.kind == xylitol_apk::PackageKind::Bundle {
            return "bundle".into();
        }
        if self.info.abis.is_empty() {
            return "base".into();
        }
        self.info.abis.join("+")
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    entries: BTreeMap<String, Entry>,
}

/// The library, backed by a JSON file on disk.
#[derive(Debug)]
pub struct Library {
    index_path: PathBuf,
    index: Index,
}

impl Library {
    /// Load the library from the default location, creating it if absent.
    pub fn open() -> anyhow::Result<Library> {
        Library::open_at(paths::library_index())
    }

    pub fn open_at(index_path: impl Into<PathBuf>) -> anyhow::Result<Library> {
        let index_path = index_path.into();
        let mut index: Index = match std::fs::read(&index_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(e.into()),
        };
        // Forget files the user deleted behind our back.
        index.entries.retain(|_, e| e.path.exists());
        Ok(Library { index_path, index })
    }

    /// Entries, newest addition first.
    pub fn entries(&self) -> Vec<&Entry> {
        let mut all: Vec<&Entry> = self.index.entries.values().collect();
        all.sort_by(|a, b| b.added.cmp(&a.added));
        all
    }

    pub fn len(&self) -> usize {
        self.index.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.entries.is_empty()
    }

    /// The first entry for a package, optionally narrowed to one version.
    ///
    /// Several entries can share a package and version — see [`Entry::key`] —
    /// so this is a lookup, not an identity.
    pub fn get(&self, package: &str, version_code: Option<i64>) -> Option<&Entry> {
        self.index.entries.values().find(|e| {
            e.info.package == package
                && version_code.is_none_or(|wanted| e.info.version_code == Some(wanted))
        })
    }

    /// Inspect `path` and record it. Replaces any entry for the same build.
    pub fn add(
        &mut self,
        path: impl AsRef<Path>,
        source: Option<Variant>,
        verified: bool,
    ) -> anyhow::Result<Entry> {
        let path = path.as_ref().to_path_buf();
        let info = xylitol_apk::inspect(&path)?;
        let entry = Entry {
            path,
            info,
            source,
            added: now_rfc3339(),
            verified,
        };
        self.index.entries.insert(entry.key(), entry.clone());
        self.save()?;
        Ok(entry)
    }

    /// Remove an entry, and optionally the file it points at.
    pub fn remove(&mut self, key: &str, delete_file: bool) -> anyhow::Result<bool> {
        let Some(entry) = self.index.entries.remove(key) else {
            return Ok(false);
        };
        if delete_file {
            let _ = std::fs::remove_file(&entry.path);
        }
        self.save()?;
        Ok(true)
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.index_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write-then-rename so a crash cannot truncate the index.
        let tmp = self.index_path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.index)?)?;
        std::fs::rename(&tmp, &self.index_path)?;
        Ok(())
    }
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_with(abis: Vec<&str>, kind: xylitol_apk::PackageKind) -> Entry {
        Entry {
            path: PathBuf::from("/x"),
            info: xylitol_apk::PackageInfo {
                kind,
                package: "com.example".into(),
                version_name: Some("1.0".into()),
                version_code: Some(7),
                label: None,
                min_sdk: None,
                target_sdk: None,
                split: None,
                abis: abis.into_iter().map(str::to_string).collect(),
                permissions: vec![],
                launchable_activities: vec![],
                contained_apks: vec![],
                file_size: 0,
                sha256: String::new(),
            },
            source: None,
            added: "2026-01-01T00:00:00Z".into(),
            verified: false,
        }
    }

    #[test]
    fn files_of_one_version_that_differ_by_abi_or_format_get_distinct_keys() {
        use xylitol_apk::PackageKind;

        let arm64 = entry_with(vec!["arm64-v8a"], PackageKind::Apk);
        let v7a = entry_with(vec!["armeabi-v7a"], PackageKind::Apk);
        let bundle = entry_with(vec![], PackageKind::Bundle);
        let plain = entry_with(vec![], PackageKind::Apk);

        assert_eq!(plain.key(), "com.example@7#base");
        assert_eq!(bundle.key(), "com.example@7#bundle");
        assert_eq!(arm64.key(), "com.example@7#arm64-v8a");

        let keys = [arm64.key(), v7a.key(), bundle.key(), plain.key()];
        let unique: std::collections::BTreeSet<&String> = keys.iter().collect();
        assert_eq!(
            unique.len(),
            4,
            "one download would replace another in the index: {keys:?}"
        );
    }

    #[test]
    fn missing_files_are_forgotten_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let index = dir.path().join("library.json");
        std::fs::write(
            &index,
            r#"{"entries":{"a@1#base":{"path":"/nonexistent/x.apk","info":{"kind":"apk",
               "package":"a","version_name":null,"version_code":1,"label":null,"min_sdk":null,
               "target_sdk":null,"split":null,"abis":[],"permissions":[],
               "launchable_activities":[],"contained_apks":[],"file_size":0,"sha256":""},
               "source":null,"added":"2026-01-01T00:00:00Z","verified":false}}}"#,
        )
        .unwrap();
        let lib = Library::open_at(&index).unwrap();
        assert!(lib.is_empty());
    }

    #[test]
    fn an_absent_index_is_an_empty_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open_at(dir.path().join("nope.json")).unwrap();
        assert_eq!(lib.len(), 0);
    }
}
