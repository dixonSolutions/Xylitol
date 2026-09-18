//! What Xylitol's own shim can do with a package.
//!
//! Xylitol does not drive an Android runtime. It loads an app's native code
//! into its own process and answers the calls that code makes — the approach
//! Cordial (<https://github.com/luohoa97/cordial>) uses to run Roblox's Android
//! build on Linux. This module is the thin layer the front-ends talk to.

use serde::{Deserialize, Serialize};

pub use xylitol_shim::abi::{self, Abi};
pub use xylitol_shim::report::{analyse, Report, Verdict};

use crate::library::Entry;

/// What the shim is capable of on this machine, independent of any app.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// The ABI whose code can be loaded here, if any.
    pub host_abi: Option<String>,
    /// Android platform libraries the shim knows the names of.
    pub known_libraries: usize,
    /// Symbols the shim implements itself.
    pub implemented_symbols: usize,
}

pub fn capability() -> Capability {
    // Count by asking the resolver, so this cannot drift from what it answers.
    let implemented = KNOWN_SHIM_SYMBOLS
        .iter()
        .filter(|name| {
            xylitol_shim::symbols::resolve([**name])
                .first()
                .map(|r| r.route == xylitol_shim::symbols::Route::Shim)
                .unwrap_or(false)
        })
        .count();

    Capability {
        host_abi: abi::host().map(|a| a.to_string()),
        known_libraries: xylitol_shim::symbols::ANDROID_SONAMES.len(),
        implemented_symbols: implemented,
    }
}

/// The names [`capability`] checks. Listing them here rather than exporting a
/// count from the resolver keeps the two able to disagree, which is what makes
/// the check meaningful.
const KNOWN_SHIM_SYMBOLS: &[&str] = &[
    "__android_log_print",
    "__android_log_write",
    "__android_log_vprint",
    "__android_log_assert",
    "__android_log_buf_write",
    "__system_property_get",
    "__system_property_find",
    "__system_property_read",
    "__system_property_read_callback",
    "__system_property_get_int64",
    "__errno",
    "gettid",
    "tgkill",
    "__progname",
];

/// Analyse a downloaded package.
pub fn check(entry: &Entry) -> anyhow::Result<Report> {
    anyhow::ensure!(
        entry.path.exists(),
        "{} is no longer on disk",
        entry.path.display()
    );
    Ok(analyse(&entry.path)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capability_report_counts_what_is_really_implemented() {
        let capability = capability();
        assert!(capability.known_libraries >= 20);
        // If this drops, something was removed from bionic.rs without the list
        // in symbols.rs being updated to match.
        assert!(
            capability.implemented_symbols >= 13,
            "only {} symbols resolve as shim",
            capability.implemented_symbols
        );
        assert!(
            capability.host_abi.is_some(),
            "tests run on a supported CPU"
        );
    }
}
