//! Where Xylitol keeps things, following the XDG base directory spec.

use std::path::PathBuf;

/// The application id, used for XDG directories and the GTK application.
pub const APP_ID: &str = "dev.xylitol.Xylitol";

/// Downloaded APK and XAPK files.
pub fn download_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("XYLITOL_DOWNLOAD_DIR") {
        return PathBuf::from(custom);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xylitol")
        .join("packages")
}

/// The library index and other mutable state.
pub fn state_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("XYLITOL_STATE_DIR") {
        return PathBuf::from(custom);
    }
    dirs::state_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xylitol")
}

/// The JSON file backing [`crate::library::Library`].
pub fn library_index() -> PathBuf {
    state_dir().join("library.json")
}
