//! Implementations of the things bionic does and glibc does not.
//!
//! Small on purpose. Every name here is one that [`crate::symbols`] reports as
//! `shim`, and a name only earns that label once something below answers it —
//! so the report cannot claim more than the code does.

use std::ffi::{c_char, c_int, CStr};
use std::sync::atomic::{AtomicBool, Ordering};

/// Android log priorities, as `android/log.h` numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Priority {
    Unknown = 0,
    Default = 1,
    Verbose = 2,
    Debug = 3,
    Info = 4,
    Warn = 5,
    Error = 6,
    Fatal = 7,
    Silent = 8,
}

impl Priority {
    fn from_raw(value: c_int) -> Priority {
        match value {
            2 => Priority::Verbose,
            3 => Priority::Debug,
            4 => Priority::Info,
            5 => Priority::Warn,
            6 => Priority::Error,
            7 => Priority::Fatal,
            8 => Priority::Silent,
            1 => Priority::Default,
            _ => Priority::Unknown,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Priority::Verbose => "V",
            Priority::Debug => "D",
            Priority::Info => "I",
            Priority::Warn => "W",
            Priority::Error => "E",
            Priority::Fatal => "F",
            Priority::Silent => "S",
            _ => "?",
        }
    }
}

static LOGGING_ENABLED: AtomicBool = AtomicBool::new(true);

/// Turn app logging on or off. On by default: an app's own log is usually the
/// only explanation available when it fails.
pub fn set_logging_enabled(enabled: bool) {
    LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Receives finished log lines from `native/log.c`.
///
/// # Safety
/// `tag` and `message` must each be either null or a valid NUL-terminated C
/// string. The C side only ever passes string literals or buffers it has just
/// NUL-terminated itself.
#[no_mangle]
pub unsafe extern "C" fn xylitol_log_line(
    priority: c_int,
    tag: *const c_char,
    message: *const c_char,
) {
    if !LOGGING_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    // SAFETY: the caller guarantees both pointers are null or NUL-terminated.
    let tag = unsafe { cstr_or(tag, "<no tag>") };
    let message = unsafe { cstr_or(message, "") };
    let priority = Priority::from_raw(priority);

    match priority {
        Priority::Error | Priority::Fatal => {
            tracing::error!(target: "app", "[{}] {tag}: {message}", priority.label())
        }
        Priority::Warn => tracing::warn!(target: "app", "[W] {tag}: {message}"),
        Priority::Silent => {}
        _ => tracing::info!(target: "app", "[{}] {tag}: {message}", priority.label()),
    }
}

/// # Safety
/// `pointer` must be null or a valid NUL-terminated C string.
unsafe fn cstr_or<'a>(pointer: *const c_char, fallback: &'a str) -> std::borrow::Cow<'a, str> {
    if pointer.is_null() {
        return std::borrow::Cow::Borrowed(fallback);
    }
    // SAFETY: guaranteed by the caller.
    unsafe { CStr::from_ptr(pointer) }.to_string_lossy()
}

/// The Android system properties an app can read.
///
/// These are not decoration: apps branch on `ro.build.version.sdk` and refuse to
/// start on an answer they do not recognise. The values describe what the shim
/// actually offers rather than impersonating a specific handset.
pub fn property(name: &str) -> Option<&'static str> {
    Some(match name {
        "ro.build.version.sdk" => "34",
        "ro.build.version.release" => "14",
        "ro.build.version.security_patch" => "2024-01-01",
        "ro.product.cpu.abi" => crate::abi::host()?.as_str(),
        "ro.product.manufacturer" => "Xylitol",
        "ro.product.model" => "Xylitol",
        "ro.product.brand" => "Xylitol",
        "ro.product.device" => "xylitol",
        "ro.product.name" => "xylitol",
        "ro.hardware" => "xylitol",
        "ro.build.fingerprint" => "Xylitol/xylitol/xylitol:14/XYL/0.1.0:user/release-keys",
        "ro.build.type" => "user",
        "ro.build.tags" => "release-keys",
        "ro.debuggable" => "0",
        "ro.secure" => "1",
        _ => return None,
    })
}

/// bionic's `__system_property_get`.
///
/// # Safety
/// `name` must be null or a valid NUL-terminated C string, and `value` must be
/// null or point to a writable buffer of at least 92 bytes — the
/// `PROP_VALUE_MAX` that Android's header promises every caller provides.
#[no_mangle]
pub unsafe extern "C" fn __system_property_get(name: *const c_char, value: *mut c_char) -> c_int {
    /// `PROP_VALUE_MAX` from `sys/system_properties.h`.
    const PROP_VALUE_MAX: usize = 92;

    if value.is_null() {
        return 0;
    }
    // SAFETY: the caller guarantees `name` is null or NUL-terminated.
    let name = unsafe { cstr_or(name, "") };
    let found = property(&name).unwrap_or("");

    // Truncate rather than overflow: the contract is a fixed-size buffer, and a
    // property longer than it is a bug here, not license to write past the end.
    let bytes = found.as_bytes();
    let length = bytes.len().min(PROP_VALUE_MAX - 1);
    // SAFETY: `value` is writable for PROP_VALUE_MAX bytes per the caller's
    // contract, and `length` is at most PROP_VALUE_MAX - 1, leaving room for the
    // terminator written below.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), value as *mut u8, length);
        *value.add(length) = 0;
    }
    length as c_int
}

/// bionic exposes `__errno`; glibc calls the same thing `__errno_location`.
#[no_mangle]
pub extern "C" fn __errno() -> *mut c_int {
    // SAFETY: __errno_location is glibc's own accessor and always returns a
    // valid pointer to this thread's errno.
    unsafe { libc::__errno_location() }
}

/// `gettid` is a bionic libc function; on glibc it is only a syscall.
#[no_mangle]
pub extern "C" fn gettid() -> libc::pid_t {
    // SAFETY: SYS_gettid takes no arguments and cannot fail.
    unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t }
}

/// `tgkill`, likewise syscall-only on glibc.
#[no_mangle]
pub extern "C" fn tgkill(tgid: libc::pid_t, tid: libc::pid_t, signal: c_int) -> c_int {
    // SAFETY: a direct syscall with three integer arguments; the kernel
    // validates them and reports EINVAL or ESRCH rather than misbehaving.
    unsafe { libc::syscall(libc::SYS_tgkill, tgid, tid, signal) as c_int }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn the_sdk_level_is_one_apps_recognise() {
        // An app that cannot parse this refuses to start, so it must be a real
        // API level rather than something invented.
        let sdk: u32 = property("ro.build.version.sdk").unwrap().parse().unwrap();
        assert!((21..=36).contains(&sdk), "implausible API level {sdk}");
    }

    #[test]
    fn an_unknown_property_is_absent_rather_than_empty() {
        assert!(property("ro.something.invented").is_none());
    }

    #[test]
    fn reading_a_property_fills_the_buffer_and_terminates_it() {
        let name = CString::new("ro.product.manufacturer").unwrap();
        let mut buffer = [0i8; 92];
        // SAFETY: a valid name and a 92-byte buffer, which is the contract.
        let written = unsafe { __system_property_get(name.as_ptr(), buffer.as_mut_ptr()) };
        assert_eq!(written, "Xylitol".len() as i32);
        // SAFETY: the call above NUL-terminated the buffer.
        let read_back = unsafe { CStr::from_ptr(buffer.as_ptr()) };
        assert_eq!(read_back.to_str().unwrap(), "Xylitol");
    }

    #[test]
    fn an_unknown_property_reads_back_as_empty() {
        let name = CString::new("ro.nope").unwrap();
        let mut buffer = [0x7fi8; 92];
        // SAFETY: as above.
        let written = unsafe { __system_property_get(name.as_ptr(), buffer.as_mut_ptr()) };
        assert_eq!(written, 0);
        assert_eq!(
            buffer[0], 0,
            "the buffer must be terminated even when empty"
        );
    }

    #[test]
    fn gettid_is_this_thread_and_errno_is_reachable() {
        assert!(gettid() > 0);
        // SAFETY: __errno returns glibc's per-thread errno pointer.
        unsafe {
            *__errno() = 0;
            assert_eq!(*__errno(), 0);
        }
    }
}
