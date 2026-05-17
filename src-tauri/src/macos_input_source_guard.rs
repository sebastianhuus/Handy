//! Reverts macOS input-source changes caused by Globe/Fn key taps.
//!
//! macOS treats a brief Fn/Globe key tap as a request to perform the
//! "Press Globe key to…" action configured in System Settings. When the
//! user has that set to "Change Input Source", every short Fn tap (e.g.
//! stopping a toggle recording, or the initial press of an Fn-first
//! chord) flips the keyboard language.
//!
//! We can't intercept the tap without root/HID-tap entitlements that
//! a regular app can't get. Instead, when an Fn-containing hotkey
//! fires we snapshot the current input source and spawn a watcher
//! thread. If the source changes within a short window, we silently
//! restore it via TISSelectInputSource.

use log::debug;
use std::ffi::c_void;
use std::thread;
use std::time::Duration;

type TISInputSourceRef = *const c_void;
type Boolean = u8;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> TISInputSourceRef;
    fn TISSelectInputSource(source: TISInputSourceRef) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFEqual(cf1: *const c_void, cf2: *const c_void) -> Boolean;
}

/// Owned TIS input source reference. TIS objects are CFType-backed and
/// must be released with CFRelease. The pointer is stored as `usize`
/// (rather than `*const c_void`) so the struct is auto-Send — Rust
/// 2021's disjoint closure captures otherwise sees through an
/// `unsafe impl Send` wrapper and flags the raw pointer.
struct OwnedInputSource(usize);

impl OwnedInputSource {
    fn from_raw(ptr: TISInputSourceRef) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(ptr as usize))
        }
    }

    fn as_ptr(&self) -> TISInputSourceRef {
        self.0 as TISInputSourceRef
    }
}

impl Drop for OwnedInputSource {
    fn drop(&mut self) {
        unsafe { CFRelease(self.as_ptr()) };
    }
}

/// How long to watch for a Globe-tap-induced input-source change.
/// The macOS tap detection completes within ~50–80 ms after key-up;
/// 200 ms gives comfortable margin without blocking anything (the
/// watcher runs on its own thread).
const REVERT_WINDOW: Duration = Duration::from_millis(200);

/// Returns true if the hotkey string includes the Fn / Globe key.
/// Cheap; called on the hot path so we avoid allocations.
pub fn hotkey_contains_fn(hotkey_string: &str) -> bool {
    hotkey_string
        .split('+')
        .any(|k| k.trim().eq_ignore_ascii_case("fn"))
}

/// Snapshot the current input source and revert any change that happens
/// within `REVERT_WINDOW`. Cheap to call from event-handling hot paths:
/// one TISCopy plus a thread spawn.
pub fn arm() {
    let Some(saved) = OwnedInputSource::from_raw(unsafe { TISCopyCurrentKeyboardInputSource() })
    else {
        return;
    };

    thread::spawn(move || {
        thread::sleep(REVERT_WINDOW);

        let Some(current) =
            OwnedInputSource::from_raw(unsafe { TISCopyCurrentKeyboardInputSource() })
        else {
            return;
        };

        let same = unsafe { CFEqual(saved.as_ptr(), current.as_ptr()) } != 0;
        if !same {
            debug!("Reverting Globe-tap-induced input-source change");
            let _ = unsafe { TISSelectInputSource(saved.as_ptr()) };
        }
    });
}
