//! Where each imported symbol comes from.
//!
//! This is the whole trick, and it is borrowed from Cordial
//! (<https://github.com/luohoa97/cordial>), which runs Roblox's Android build
//! this way. Every name an Android object imports resolves one of three ways:
//!
//! | route | meaning |
//! |---|---|
//! | **shim** | Xylitol implements it, because bionic or the framework does something glibc does not |
//! | **host** | forwarded straight to the host's own libc, because the two agree |
//! | **stub** | not implemented, and it *reports failure* rather than faking success |
//!
//! That last row is a rule rather than an accident. A stub that returns success
//! sends the app off on an answer that is not true, and it then fails somewhere
//! with no relationship to the cause. A stub that says "unsupported" fails where
//! the problem is.

use std::collections::BTreeMap;
use std::ffi::CString;

use serde::{Deserialize, Serialize};

/// How one symbol is satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Route {
    /// Implemented here.
    Shim,
    /// Forwarded to the host's libc.
    Host,
    /// Not implemented. Calling it fails loudly.
    Stub,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Route::Shim => "shim",
            Route::Host => "host",
            Route::Stub => "stub",
        }
    }
}

/// The Android shared libraries an APK's native code may link against.
///
/// None of these exist on a Linux desktop. Each is a *virtual* library as far as
/// the shim is concerned: a name the loader must satisfy from its own table.
pub const ANDROID_SONAMES: &[&str] = &[
    "libc.so",
    "libm.so",
    "libdl.so",
    "libstdc++.so",
    "libz.so",
    "liblog.so",
    "libandroid.so",
    "libnativewindow.so",
    "libEGL.so",
    "libGLESv1_CM.so",
    "libGLESv2.so",
    "libGLESv3.so",
    "libvulkan.so",
    "libjnigraphics.so",
    "libOpenSLES.so",
    "libOpenMAXAL.so",
    "libaaudio.so",
    "libmediandk.so",
    "libcamera2ndk.so",
    "libbinder_ndk.so",
    "libnativehelper.so",
    "libandroid_runtime.so",
    "libneuralnetworks.so",
    "libsync.so",
];

/// Whether a `DT_NEEDED` name is one the platform is expected to provide.
///
/// Anything else ships inside the APK next to the object that needs it.
pub fn is_android_soname(name: &str) -> bool {
    ANDROID_SONAMES.contains(&name)
}

/// Symbols Xylitol implements itself, and what each belongs to.
///
/// Kept as data rather than inferred, so the report cannot drift from reality:
/// a name is only listed here once something in [`crate::bionic`] actually
/// answers it.
fn implemented() -> BTreeMap<&'static str, &'static str> {
    let mut map = BTreeMap::new();
    for name in [
        "__android_log_print",
        "__android_log_write",
        "__android_log_vprint",
        "__android_log_assert",
        "__android_log_buf_write",
    ] {
        map.insert(name, "liblog.so");
    }
    for name in [
        "__system_property_get",
        "__system_property_find",
        "__system_property_read",
        "__system_property_read_callback",
        "__system_property_get_int64",
    ] {
        map.insert(name, "libc.so (bionic properties)");
    }
    for name in ["__errno", "gettid", "tgkill", "__progname"] {
        map.insert(name, "libc.so (bionic deltas)");
    }
    map
}

/// The decision made about one symbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub symbol: String,
    pub route: Route,
    /// For a shim symbol, which virtual library it belongs to.
    pub provider: Option<String>,
}

/// Classify every symbol in `symbols`.
pub fn resolve<'a>(symbols: impl IntoIterator<Item = &'a str>) -> Vec<Resolution> {
    let implemented = implemented();
    let mut out: Vec<Resolution> = symbols
        .into_iter()
        .map(|symbol| {
            if let Some(provider) = implemented.get(symbol) {
                return Resolution {
                    symbol: symbol.to_string(),
                    route: Route::Shim,
                    provider: Some((*provider).to_string()),
                };
            }
            // Ask the host rather than keeping a list of glibc's exports: the
            // list would be wrong on some distribution sooner or later, and the
            // real question is whether *this* machine can satisfy the name.
            if host_provides(symbol) {
                return Resolution {
                    symbol: symbol.to_string(),
                    route: Route::Host,
                    provider: Some("host libc".to_string()),
                };
            }
            Resolution {
                symbol: symbol.to_string(),
                route: Route::Stub,
                provider: None,
            }
        })
        .collect();
    out.sort_by(|a, b| a.symbol.cmp(&b.symbol));
    out
}

/// Whether the host's own libraries already export `symbol`.
pub fn host_provides(symbol: &str) -> bool {
    let Ok(name) = CString::new(symbol) else {
        return false;
    };
    // SAFETY: `name` is a valid NUL-terminated C string that outlives the call,
    // and RTLD_DEFAULT searches already-loaded objects without loading any.
    // dlsym returns null for a name it cannot find, which is the answer wanted
    // here; no returned pointer is dereferenced.
    let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    !address.is_null()
}

/// A count of how the symbols of one object split across the three routes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteCounts {
    pub shim: usize,
    pub host: usize,
    pub stub: usize,
}

impl RouteCounts {
    pub fn of(resolutions: &[Resolution]) -> RouteCounts {
        let mut counts = RouteCounts::default();
        for resolution in resolutions {
            match resolution.route {
                Route::Shim => counts.shim += 1,
                Route::Host => counts.host += 1,
                Route::Stub => counts.stub += 1,
            }
        }
        counts
    }

    pub fn total(self) -> usize {
        self.shim + self.host + self.stub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_libc_names_come_from_the_host() {
        assert!(host_provides("malloc"));
        assert!(host_provides("memcpy"));
        assert!(!host_provides(
            "a_symbol_that_does_not_exist_anywhere_xyzzy"
        ));
    }

    #[test]
    fn android_only_names_are_shimmed_not_borrowed() {
        let resolved = resolve(["__android_log_print", "__system_property_get"]);
        assert!(
            resolved.iter().all(|r| r.route == Route::Shim),
            "{resolved:?}"
        );
    }

    #[test]
    fn an_unknown_name_is_a_stub_rather_than_a_guess() {
        let resolved = resolve(["ASomething_thatDoesNotExist"]);
        assert_eq!(resolved[0].route, Route::Stub);
        assert!(resolved[0].provider.is_none());
    }

    #[test]
    fn the_three_routes_account_for_everything() {
        let resolved = resolve(["malloc", "__android_log_print", "AMidiDevice_fromJava"]);
        let counts = RouteCounts::of(&resolved);
        assert_eq!(counts.total(), 3);
        assert_eq!(
            counts,
            RouteCounts {
                shim: 1,
                host: 1,
                stub: 1
            }
        );
    }

    #[test]
    fn platform_sonames_are_recognised_and_app_ones_are_not() {
        assert!(is_android_soname("libandroid.so"));
        assert!(is_android_soname("libEGL.so"));
        assert!(!is_android_soname("libroblox.so"));
        assert!(!is_android_soname("libtermux.so"));
    }
}
