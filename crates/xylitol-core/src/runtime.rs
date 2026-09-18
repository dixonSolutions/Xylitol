//! Handing a package to an Android runtime.
//!
//! Shashlik shipped its own AOSP-derived runtime. That approach died with the
//! Android 4.4 tree it was built against, so Xylitol does not try to revive it:
//! it drives whichever runtime is already on the system. Waydroid is the direct
//! successor for the desktop; plain `adb` covers a connected device or emulator.

use std::path::{Path, PathBuf};

use crate::library::Entry;

/// A way of getting a package onto an Android system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Waydroid, a containerised Android on the Wayland session.
    Waydroid,
    /// Any device or emulator reachable over adb.
    Adb,
}

impl Backend {
    pub fn command(self) -> &'static str {
        match self {
            Backend::Waydroid => "waydroid",
            Backend::Adb => "adb",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Waydroid => "Waydroid",
            Backend::Adb => "adb device",
        }
    }
}

/// What a backend can do right now.
#[derive(Debug, Clone)]
pub struct Status {
    pub backend: Backend,
    /// The binary was found on `PATH`.
    pub installed: bool,
    /// The runtime answered, so an install would be attempted for real.
    pub ready: bool,
    /// A short line to show the user — a version, or why it is not ready.
    pub detail: String,
}

/// Probe every backend Xylitol knows about.
pub async fn detect() -> Vec<Status> {
    let mut out = Vec::new();
    for backend in [Backend::Waydroid, Backend::Adb] {
        out.push(probe(backend).await);
    }
    out
}

async fn probe(backend: Backend) -> Status {
    let Some(_) = which(backend.command()) else {
        return Status {
            backend,
            installed: false,
            ready: false,
            detail: format!("`{}` is not installed", backend.command()),
        };
    };

    let (args, ready_if): (&[&str], fn(&str) -> bool) = match backend {
        // `waydroid status` prints `Session:\tRUNNING` once the container is up.
        Backend::Waydroid => (&["status"], |out| out.contains("RUNNING")),
        // `adb devices` lists one line per attached device after the header.
        Backend::Adb => (&["devices"], |out| {
            out.lines()
                .skip(1)
                .any(|l| l.trim_end().ends_with("device"))
        }),
    };

    match run(backend.command(), args).await {
        Ok(output) => {
            let ready = ready_if(&output);
            Status {
                backend,
                installed: true,
                ready,
                detail: if ready {
                    first_useful_line(&output)
                } else {
                    match backend {
                        Backend::Waydroid => "installed, but no session is running".into(),
                        Backend::Adb => "installed, but no device is connected".into(),
                    }
                },
            }
        }
        Err(e) => Status {
            backend,
            installed: true,
            ready: false,
            detail: e.to_string(),
        },
    }
}

/// Install a library entry through `backend`.
///
/// Returns the runtime's own output so the caller can show it verbatim; these
/// tools report failures in ways that are more useful than an exit code.
pub async fn install(backend: Backend, entry: &Entry) -> anyhow::Result<String> {
    let path = &entry.path;
    anyhow::ensure!(path.exists(), "{} is no longer on disk", path.display());

    if entry.info.kind == xylitol_apk::PackageKind::Bundle {
        return install_bundle(backend, path).await;
    }

    let path_str = path.to_string_lossy().to_string();
    let args: Vec<&str> = match backend {
        Backend::Waydroid => vec!["app", "install", &path_str],
        Backend::Adb => vec!["install", "-r", &path_str],
    };
    run(backend.command(), &args).await
}

/// A bundle is a zip of split APKs that must be installed as one transaction.
async fn install_bundle(backend: Backend, path: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(
        backend == Backend::Adb,
        "installing an XAPK bundle needs adb: it must install every split in one \
         transaction, which `waydroid app install` cannot do. Start Waydroid and \
         connect to it with adb, or pick a plain APK variant instead."
    );

    let dir = tempdir_for(path)?;
    let apks = extract_apks(path, &dir)?;
    anyhow::ensure!(!apks.is_empty(), "this bundle contains no APK files");

    let mut args: Vec<String> = vec!["install-multiple".into(), "-r".into()];
    args.extend(apks.iter().map(|p| p.to_string_lossy().to_string()));
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();

    let result = run("adb", &arg_refs).await;
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn tempdir_for(path: &Path) -> anyhow::Result<PathBuf> {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let dir = std::env::temp_dir().join(format!("xylitol-{stem}-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn extract_apks(bundle: &Path, dest: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let info = xylitol_apk::inspect(bundle)?;
    let file = std::fs::File::open(bundle)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let mut out = Vec::new();
    for name in &info.contained_apks {
        let mut entry = zip.by_name(name)?;
        // Flatten: split APKs may sit in subdirectories inside the bundle.
        let leaf = Path::new(name).file_name().unwrap_or_default();
        let target = dest.join(leaf);
        let mut file = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut file)?;
        out.push(target);
    }
    Ok(out)
}

/// Launch an installed app.
pub async fn launch(
    backend: Backend,
    package: &str,
    activity: Option<&str>,
) -> anyhow::Result<String> {
    match backend {
        Backend::Waydroid => run("waydroid", &["app", "launch", package]).await,
        Backend::Adb => {
            let component;
            let args: Vec<&str> = match activity {
                Some(activity) => {
                    component = format!("{package}/{activity}");
                    vec!["shell", "am", "start", "-n", &component]
                }
                None => vec![
                    "shell",
                    "monkey",
                    "-p",
                    package,
                    "-c",
                    "android.intent.category.LAUNCHER",
                    "1",
                ],
            };
            run("adb", &args).await
        }
    }
}

async fn run(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("could not run `{program}`: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = [stdout, stderr]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if output.status.success() {
        Ok(combined)
    } else {
        Err(anyhow::anyhow!(
            "`{program} {}` failed: {}",
            args.join(" "),
            if combined.is_empty() {
                "no output".into()
            } else {
                combined
            }
        ))
    }
}

fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn first_useful_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("ready")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_finds_a_real_binary_and_not_a_fake_one() {
        assert!(which("sh").is_some());
        assert!(which("definitely-not-a-real-binary-xyzzy").is_none());
    }

    #[tokio::test]
    async fn a_missing_backend_reports_itself_as_not_installed() {
        // `detect` must never panic or hang on a machine with neither tool.
        for status in detect().await {
            if !status.installed {
                assert!(!status.ready);
                assert!(status.detail.contains("not installed"));
            }
        }
    }
}
