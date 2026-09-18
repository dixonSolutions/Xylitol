//! Can this app run here, and if not, what is missing?
//!
//! Running an Android app without an Android runtime means the shim has to
//! satisfy everything the app's native code imports. Whether that is possible is
//! decidable before anything is executed, by reading the objects — which is what
//! this does. It answers honestly, including "no".

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::abi::{self, Abi};
use crate::elf::{self, EntryPoint, NativeObject};
use crate::symbols::{self, Resolution, Route, RouteCounts};
use crate::Error;

/// What the shim can do with an app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "verdict", content = "detail")]
pub enum Verdict {
    /// Native code for this machine, a self-sufficient entry point, and every
    /// symbol accounted for. Nothing here has been executed, so this says the
    /// app is a candidate, not that it works.
    Candidate,
    /// Runnable in shape, but some symbols have no implementation. Those calls
    /// will fail where they are made.
    MissingSymbols { count: usize },
    /// The app ships no native code for this machine's instruction set.
    WrongAbi {
        host: String,
        available: Vec<String>,
    },
    /// The app has no native code at all: it is Java or Kotlin through and
    /// through, and needs a DEX interpreter and the framework classes.
    NoNativeCode,
    /// There is native code, but nothing that can start on its own — only
    /// `JNI_OnLoad`, which a Java VM is supposed to call.
    NeedsJavaRuntime { jni_natives: usize },
}

impl Verdict {
    /// Whether the shim should attempt to run this at all.
    pub fn is_runnable(&self) -> bool {
        matches!(self, Verdict::Candidate | Verdict::MissingSymbols { .. })
    }

    pub fn headline(&self) -> String {
        match self {
            Verdict::Candidate => "every imported symbol is accounted for".into(),
            Verdict::MissingSymbols { count } => {
                format!("{count} symbol(s) have no implementation and will fail when called")
            }
            Verdict::WrongAbi { host, available } => format!(
                "no native code for {host}; this app ships {}",
                if available.is_empty() {
                    "none".to_string()
                } else {
                    available.join(", ")
                }
            ),
            Verdict::NoNativeCode => {
                "no native code at all — this app is Java or Kotlin, which needs a DEX \
                 interpreter and the android.* framework classes"
                    .into()
            }
            Verdict::NeedsJavaRuntime { jni_natives } => format!(
                "native code exists but only as a JNI library ({jni_natives} Java_* entry \
                 point(s)); something has to run the app's Java before any of it is called"
            ),
        }
    }
}

/// The full analysis of one app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub package: String,
    pub version: String,
    pub host_abi: Option<String>,
    /// ABIs the app ships, whether usable here or not.
    pub available_abis: Vec<String>,
    pub objects: Vec<ObjectReport>,
    /// Android platform libraries the app links against, across all objects.
    pub android_libraries: Vec<String>,
    /// Every symbol with no implementation, deduplicated across objects.
    pub unimplemented: Vec<String>,
    pub totals: RouteCounts,
    pub verdict: Verdict,
}

/// One native object's share of the analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectReport {
    pub object: NativeObject,
    pub counts: RouteCounts,
    pub resolutions: Vec<Resolution>,
}

/// Analyse an APK or bundle.
pub fn analyse(path: impl AsRef<Path>) -> Result<Report, Error> {
    let path = path.as_ref();
    let info = xylitol_apk::inspect(path).map_err(|e| Error::Apk(e.to_string()))?;
    let host = abi::host();

    let libraries = native_libraries(path)?;
    let available: BTreeSet<String> = libraries.keys().map(|(abi, _)| abi.to_string()).collect();

    // Only objects for this machine's instruction set can be loaded; there is no
    // translation layer, by design.
    let usable: Vec<(&String, &Vec<u8>)> = match host {
        Some(host) => libraries
            .iter()
            .filter(|((abi, _), _)| *abi == host)
            .map(|((_, name), data)| (name, data))
            .collect(),
        None => Vec::new(),
    };

    let mut objects = Vec::new();
    let mut android_libraries = BTreeSet::new();
    let mut unimplemented = BTreeSet::new();
    let mut totals = RouteCounts::default();
    let mut entry_points = BTreeSet::new();
    let mut jni_natives = 0usize;

    for (name, data) in &usable {
        let object = elf::read(name, data)?;
        for needed in &object.needed {
            if symbols::is_android_soname(needed) {
                android_libraries.insert(needed.clone());
            }
        }
        entry_points.extend(object.entry_points.iter().copied());
        jni_natives += object.jni_natives;

        let resolutions = symbols::resolve(object.undefined.iter().map(String::as_str));
        for resolution in &resolutions {
            if resolution.route == Route::Stub {
                unimplemented.insert(resolution.symbol.clone());
            }
        }
        let counts = RouteCounts::of(&resolutions);
        totals.shim += counts.shim;
        totals.host += counts.host;
        totals.stub += counts.stub;

        objects.push(ObjectReport {
            object,
            counts,
            resolutions,
        });
    }

    let verdict = decide(
        host,
        &available,
        &objects,
        &entry_points,
        jni_natives,
        unimplemented.len(),
    );

    let version = info.version_display();
    Ok(Report {
        package: info.package,
        version,
        host_abi: host.map(|a| a.to_string()),
        available_abis: available.into_iter().collect(),
        objects,
        android_libraries: android_libraries.into_iter().collect(),
        unimplemented: unimplemented.into_iter().collect(),
        totals,
        verdict,
    })
}

fn decide(
    host: Option<Abi>,
    available: &BTreeSet<String>,
    objects: &[ObjectReport],
    entry_points: &BTreeSet<EntryPoint>,
    jni_natives: usize,
    unimplemented: usize,
) -> Verdict {
    if available.is_empty() {
        return Verdict::NoNativeCode;
    }
    if objects.is_empty() {
        return Verdict::WrongAbi {
            host: host
                .map(|a| a.to_string())
                .unwrap_or_else(|| "unknown".into()),
            available: available.iter().cloned().collect(),
        };
    }
    if !entry_points.iter().any(|e| e.is_self_sufficient()) {
        return Verdict::NeedsJavaRuntime { jni_natives };
    }
    if unimplemented > 0 {
        return Verdict::MissingSymbols {
            count: unimplemented,
        };
    }
    Verdict::Candidate
}

/// Pull every `lib/<abi>/*.so` out of an APK, or out of each APK in a bundle.
fn native_libraries(path: &Path) -> Result<BTreeMap<(Abi, String), Vec<u8>>, Error> {
    let file = std::fs::File::open(path).map_err(|e| Error::Apk(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Apk(e.to_string()))?;

    let mut out = BTreeMap::new();
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();

    for name in &names {
        if let Some((abi, _)) = split_lib_path(name) {
            let mut entry = zip.by_name(name).map_err(|e| Error::Apk(e.to_string()))?;
            let mut data = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut data)
                .map_err(|e| Error::Apk(e.to_string()))?;
            out.insert((abi, name.clone()), data);
        }
    }

    // A bundle keeps its native code in the split APKs inside it.
    if out.is_empty() {
        for name in &names {
            if !name.to_ascii_lowercase().ends_with(".apk") {
                continue;
            }
            let mut entry = zip.by_name(name).map_err(|e| Error::Apk(e.to_string()))?;
            let mut inner = Vec::with_capacity(entry.size() as usize);
            entry
                .read_to_end(&mut inner)
                .map_err(|e| Error::Apk(e.to_string()))?;
            drop(entry);

            let mut inner_zip = match zip::ZipArchive::new(std::io::Cursor::new(inner)) {
                Ok(zip) => zip,
                Err(_) => continue,
            };
            let inner_names: Vec<String> = inner_zip.file_names().map(str::to_string).collect();
            for inner_name in inner_names {
                let Some((abi, _)) = split_lib_path(&inner_name) else {
                    continue;
                };
                let Ok(mut file) = inner_zip.by_name(&inner_name) else {
                    continue;
                };
                let mut data = Vec::with_capacity(file.size() as usize);
                if file.read_to_end(&mut data).is_ok() {
                    out.insert((abi, format!("{name}!{inner_name}")), data);
                }
            }
        }
    }

    Ok(out)
}

fn split_lib_path(name: &str) -> Option<(Abi, &str)> {
    let rest = name.strip_prefix("lib/")?;
    let (abi, file) = rest.split_once('/')?;
    if !file.ends_with(".so") {
        return None;
    }
    Some((Abi::parse(abi)?, file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_are_recognised_by_shape() {
        assert_eq!(
            split_lib_path("lib/x86_64/libfoo.so"),
            Some((Abi::X86_64, "libfoo.so"))
        );
        assert!(split_lib_path("lib/x86_64/notalib.txt").is_none());
        assert!(split_lib_path("assets/lib/x86_64/libfoo.so").is_none());
        assert!(split_lib_path("lib/mips/libfoo.so").is_none());
    }

    #[test]
    fn an_app_with_no_native_code_says_why() {
        let verdict = decide(
            Some(Abi::X86_64),
            &BTreeSet::new(),
            &[],
            &BTreeSet::new(),
            0,
            0,
        );
        assert_eq!(verdict, Verdict::NoNativeCode);
        assert!(!verdict.is_runnable());
        assert!(verdict.headline().contains("DEX"));
    }

    #[test]
    fn native_code_for_another_cpu_is_not_pretended_to_work() {
        let available = ["arm64-v8a".to_string()].into_iter().collect();
        let verdict = decide(Some(Abi::X86_64), &available, &[], &BTreeSet::new(), 0, 0);
        match verdict {
            Verdict::WrongAbi { host, available } => {
                assert_eq!(host, "x86_64");
                assert_eq!(available, vec!["arm64-v8a"]);
            }
            other => panic!("expected WrongAbi, got {other:?}"),
        }
    }

    /// A loadable object, so `decide` reaches past its earlier arms.
    fn an_object() -> ObjectReport {
        ObjectReport {
            object: NativeObject {
                path: "lib/x86_64/libapp.so".into(),
                soname: Some("libapp.so".into()),
                abi: Abi::X86_64,
                needed: vec!["libc.so".into()],
                undefined: vec!["malloc".into()],
                entry_points: vec![],
                jni_natives: 42,
                file_size: 1024,
            },
            counts: RouteCounts {
                shim: 0,
                host: 1,
                stub: 0,
            },
            resolutions: vec![],
        }
    }

    #[test]
    fn a_jni_only_library_needs_something_to_call_it() {
        let available = ["x86_64".to_string()].into_iter().collect();
        let entry_points = [EntryPoint::JniOnLoad].into_iter().collect();
        let verdict = decide(
            Some(Abi::X86_64),
            &available,
            &[an_object()],
            &entry_points,
            42,
            0,
        );
        assert_eq!(verdict, Verdict::NeedsJavaRuntime { jni_natives: 42 });
        assert!(!verdict.is_runnable());
        assert!(verdict.headline().contains("Java"));
    }

    #[test]
    fn a_native_activity_with_everything_resolved_is_a_candidate() {
        let available = ["x86_64".to_string()].into_iter().collect();
        let entry_points = [EntryPoint::NativeActivity].into_iter().collect();
        let verdict = decide(
            Some(Abi::X86_64),
            &available,
            &[an_object()],
            &entry_points,
            0,
            0,
        );
        assert_eq!(verdict, Verdict::Candidate);
        assert!(verdict.is_runnable());
    }

    #[test]
    fn unimplemented_symbols_are_reported_rather_than_hidden() {
        let available = ["x86_64".to_string()].into_iter().collect();
        let entry_points = [EntryPoint::NativeActivity].into_iter().collect();
        let verdict = decide(
            Some(Abi::X86_64),
            &available,
            &[an_object()],
            &entry_points,
            0,
            7,
        );
        assert_eq!(verdict, Verdict::MissingSymbols { count: 7 });
        // Still worth attempting: the app may never call any of them.
        assert!(verdict.is_runnable());
        assert!(verdict.headline().contains('7'));
    }
}
