// macOS IME mode detection via Text Input Source (TIS) Carbon framework.
//
// Strategy: poll TISCopyCurrentKeyboardInputSource() and check if the input source ID
// contains "inputmethod.Japanese". Covers:
//   - com.apple.inputmethod.Kotoeri.RomajiTyping.Japanese  (Apple IME, romaji)
//   - com.apple.inputmethod.Kotoeri.KanaTyping.Japanese    (Apple IME, kana)
//   - com.google.inputmethod.Japanese.base                 (Google Japanese IME)
//
// Thread-safety: TIS functions (TSMGetInputSourceProperty) assert they are called on
// the main GCD queue (dispatch_assert_queue). Calling from a background thread causes
// a SIGTRAP (EXC_BREAKPOINT) via _dispatch_assert_queue_fail.
//
// Fix: dispatch_sync_f marshals the TIS call to the main queue from any calling thread.
// The polling thread blocks for the duration of the main-thread call (~microseconds).
// No deadlock risk: the main thread never waits on the polling thread.
//
// Limitation: does not detect active composition (romaji→hiragana in progress).
// IME_ACTIVE remains false on macOS; HMM runs during candidate selection.

use std::ffi::c_void;

use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::CFString;

// ---------------------------------------------------------------------------
// TIS FFI declarations (Carbon framework, pre-installed on all macOS systems)
// ---------------------------------------------------------------------------
#[allow(non_camel_case_types)]
type TISInputSourceRef = *const c_void;
#[allow(non_camel_case_types)]
type CFStringRef = *const c_void;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> TISInputSourceRef;
    fn TISGetInputSourceProperty(source: TISInputSourceRef, key: CFStringRef) -> *const c_void;
    #[allow(non_upper_case_globals)]
    static kTISPropertyInputSourceID: CFStringRef;
}

// ---------------------------------------------------------------------------
// libdispatch FFI — dispatch_sync_f marshals work onto the main GCD queue.
// Blocks the calling thread until the work runs; safe from background threads.
//
// dispatch_get_main_queue() is a C static inline that expands to the global
// symbol `_dispatch_main_q`, NOT an exported function. We link the global
// directly and pass a pointer to it as the queue argument.
// ---------------------------------------------------------------------------
#[link(name = "System", kind = "dylib")]
extern "C" {
    /// The main dispatch queue global — the target of dispatch_get_main_queue().
    #[allow(non_upper_case_globals)]
    static _dispatch_main_q: c_void;

    fn dispatch_sync_f(
        queue: *const c_void,
        context: *mut c_void,
        work: unsafe extern "C" fn(*mut c_void),
    );
}

// ---------------------------------------------------------------------------
// TIS work function — executed on the main thread by dispatch_sync_f
// ---------------------------------------------------------------------------

/// Context struct passed through the void* context pointer.
struct TisQueryCtx {
    result: bool,
}

/// C-compatible callback that performs the TIS query on the main thread.
unsafe extern "C" fn tis_query_main_thread(ctx: *mut c_void) {
    let ctx = &mut *(ctx as *mut TisQueryCtx);

    let source = TISCopyCurrentKeyboardInputSource();
    if source.is_null() {
        ctx.result = false;
        return;
    }

    let id_ref = TISGetInputSourceProperty(source, kTISPropertyInputSourceID);

    ctx.result = if id_ref.is_null() {
        false
    } else {
        // wrap_under_get_rule: TIS owns the string ref; we must not release it
        let cf_str = CFString::wrap_under_get_rule(id_ref as core_foundation::string::CFStringRef);
        let id_str = cf_str.to_string();
        // Match all known Japanese IME input source IDs
        id_str.contains("inputmethod.Japanese")
    };

    // Release the *source* object returned by TISCopyCurrentKeyboard… (Copy rule)
    CFRelease(source);
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns true if the current system keyboard input source is a Japanese IME.
///
/// Safe to call from **any thread** — internally dispatches to the main GCD queue
/// (required by HIToolbox) via `dispatch_sync_f` and blocks until the result is ready.
pub fn is_japanese_ime_open() -> bool {
    let mut ctx = TisQueryCtx { result: false };
    unsafe {
        dispatch_sync_f(
            &_dispatch_main_q as *const c_void,
            &mut ctx as *mut TisQueryCtx as *mut c_void,
            tis_query_main_thread,
        );
    }
    ctx.result
}
