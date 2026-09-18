//! Shared logic behind both Xylitol front-ends.
//!
//! The GUI and the CLI are thin: everything they do — searching APKPure,
//! choosing a build, downloading it, reading what it contains and deciding
//! whether Xylitol's own shim can run it — lives here.

pub mod library;
pub mod paths;
pub mod shim;

pub use xylitol_apk as apk;
pub use xylitol_apkpure as apkpure;
pub use xylitol_shim as shim_core;

/// Format a byte count the way the UI shows it.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_size;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(47_990_842), "45.8 MB");
    }
}
