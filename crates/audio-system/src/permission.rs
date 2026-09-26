//! The system audio permission (R9, R10, KTD12).
//!
//! macOS has no public API for the "Screen & System Audio Recording"
//! permission, and a tap without it records silence and returns no error.
//! So [`preflight`] loads `TCC.framework` at run time and calls its private
//! `TCCAccessPreflight` function for `kTCCServiceAudioCapture`, as AudioCap
//! does (see PROVENANCE.md). When the function is missing, the result is
//! [`Permission::Unknown`], so a macOS change cannot stop a recording.
//! [`request`] opens a short tap, which makes macOS ask the user.
//!
//! Windows has no such permission: the result is always allowed.

use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::Instant;

/// The state of the system audio permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    Denied,
    /// macOS has not asked the user yet, or its prompt waits for an answer.
    NotAsked,
    /// The check could not run: the private function is missing, or it gave
    /// a result that Longhand does not know.
    Unknown,
}

/// How long [`request`] keeps its tap open.
pub const REQUEST_TAP: Duration = Duration::from_millis(500);

/// The time between two cancellation checks in [`request_unless`].
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const REQUEST_STEP: Duration = Duration::from_millis(50);

/// The raw results of `TCCAccessPreflight` (AudioCap, anarlog's `tcc`).
const RAW_GRANTED: i32 = 0;
const RAW_DENIED: i32 = 1;
const RAW_NOT_ASKED: i32 = 2;

/// The preflight function after it loads: one call returns its raw result
/// for the system audio service.
pub(crate) type Preflight = Box<dyn FnOnce() -> i32>;

fn from_raw(raw: i32) -> Permission {
    match raw {
        RAW_GRANTED => Permission::Allowed,
        RAW_DENIED => Permission::Denied,
        RAW_NOT_ASKED => Permission::NotAsked,
        other => {
            log::warn!("the system audio permission check returned {other}; the status is unknown");
            Permission::Unknown
        }
    }
}

/// The check with a loader that the caller gives. The tests replace the
/// loader to simulate a macOS that does not have the function. Only macOS
/// loads a function; the other platforms use the tests only.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn preflight_with(load: impl FnOnce() -> Option<Preflight>) -> Permission {
    match load() {
        Some(call) => from_raw(call()),
        None => Permission::Unknown,
    }
}

/// The state of the system audio permission. It asks the user nothing and
/// takes about a millisecond.
pub fn preflight() -> Permission {
    #[cfg(target_os = "macos")]
    {
        preflight_with(tcc::load)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Permission::Allowed
    }
}

/// Ask for the permission: open a tap for [`REQUEST_TAP`], stop it, and
/// check again. On macOS the first tap shows the system prompt. The prompt
/// does not wait for the tap, so the result can be
/// [`Permission::NotAsked`] while the prompt is on the screen: poll
/// [`preflight`] after it. It blocks for about half a second, so call it
/// away from a command's thread and the segment thread (KTD12).
pub fn request() -> Permission {
    request_unless(|| false)
}

/// [`request`] that stops the tap early when `cancelled` becomes true, for
/// example on Stop (KTD12).
pub fn request_unless(cancelled: impl Fn() -> bool) -> Permission {
    #[cfg(target_os = "macos")]
    {
        match crate::open(0, 480) {
            Ok(capture) => {
                let until = Instant::now() + REQUEST_TAP;
                while !cancelled() && Instant::now() < until {
                    std::thread::sleep(REQUEST_STEP);
                }
                drop(capture);
            }
            Err(e) => log::warn!("the permission tap did not open: {e}"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cancelled;
    }
    preflight()
}

/// The private TCC function, loaded once through `dlopen`.
#[cfg(target_os = "macos")]
mod tcc {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::OnceLock;

    use cidre::cf;

    use super::Preflight;

    const TCC_PATH: &CStr = c"/System/Library/PrivateFrameworks/TCC.framework/Versions/A/TCC";
    const SYMBOL: &CStr = c"TCCAccessPreflight";
    const SERVICE: &str = "kTCCServiceAudioCapture";
    const RTLD_NOW: c_int = 0x2;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// `int TCCAccessPreflight(CFStringRef service, CFDictionaryRef options)`.
    type PreflightFn = unsafe extern "C" fn(service: &cf::String, options: *const c_void) -> c_int;

    fn find() -> Option<PreflightFn> {
        // SAFETY: both strings end in a NUL. The handle stays open for the
        // life of the process, so the function pointer stays valid.
        unsafe {
            let handle = dlopen(TCC_PATH.as_ptr(), RTLD_NOW);
            if handle.is_null() {
                log::warn!("TCC.framework did not load; the system audio permission is unknown");
                return None;
            }
            let symbol = dlsym(handle, SYMBOL.as_ptr());
            if symbol.is_null() {
                log::warn!(
                    "TCC.framework has no TCCAccessPreflight; the system audio permission is unknown"
                );
                return None;
            }
            Some(std::mem::transmute::<*mut c_void, PreflightFn>(symbol))
        }
    }

    pub(super) fn load() -> Option<Preflight> {
        static FOUND: OnceLock<Option<PreflightFn>> = OnceLock::new();
        let preflight = (*FOUND.get_or_init(find))?;
        Some(Box::new(move || {
            let service = cf::String::from_str(SERVICE);
            // SAFETY: the function takes a CFString and an optional
            // dictionary, and it keeps neither after it returns.
            unsafe { preflight(&service, std::ptr::null()) }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_function_gives_unknown() {
        assert_eq!(preflight_with(|| None), Permission::Unknown);
    }

    #[test]
    fn each_raw_result_maps_to_its_state() {
        let with = |raw: i32| preflight_with(move || Some(Box::new(move || raw)));
        assert_eq!(with(0), Permission::Allowed);
        assert_eq!(with(1), Permission::Denied);
        assert_eq!(with(2), Permission::NotAsked);
        assert_eq!(with(-1), Permission::Unknown);
        assert_eq!(with(7), Permission::Unknown);
    }

    /// The real loader finds the private function on this macOS. When a
    /// macOS update removes it, this test fails first, and the app shows
    /// "Cannot check" in place of a wrong state.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_private_function_loads_on_this_macos() {
        assert!(tcc::load().is_some());
        assert_ne!(preflight(), Permission::Unknown);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_have_no_permission_to_ask_for() {
        assert_eq!(preflight(), Permission::Allowed);
        assert_eq!(request(), Permission::Allowed);
    }
}
