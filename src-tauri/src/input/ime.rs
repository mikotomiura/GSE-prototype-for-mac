use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use core_foundation::base::{CFRelease, TCFType};
use core_foundation::string::CFString;
use crossbeam_channel::Sender;

use crate::logger::LogEntry;

// ---------------------------------------------------------------------------
// Platform-specific IME monitor implementations
// ---------------------------------------------------------------------------

#[path = "ime_macos.rs"]
pub mod ime_macos;

// ---------------------------------------------------------------------------
// CGWindowList FFI — candidate window detection via window enumeration.
//
// Strategy: enumerate on-screen windows via CGWindowListCopyWindowInfo and
// check if any belong to a known Japanese IME process at a non-zero window
// layer (candidate / overlay windows).
//
// Permission: kCGWindowOwnerName does NOT require Screen Recording.
// Only kCGWindowName (window title) is gated, and we do not read it.
//
// Thread safety: CGWindowListCopyWindowInfo is safe to call from any thread.
// ---------------------------------------------------------------------------

/// CGWindowListOption: only on-screen windows.
const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
/// Null window ID — enumerate all windows.
const K_CG_NULL_WINDOW_ID: u32 = 0;
/// kCFNumberSInt32Type
const K_CF_NUMBER_SINT32_TYPE: u32 = 3;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: u32) -> *const c_void;

    /// Window owner process name (CFStringRef). NOT gated by Screen Recording.
    #[allow(non_upper_case_globals)]
    static kCGWindowOwnerName: *const c_void;
    /// Window layer number (CFNumberRef, i32). Layer 0 = normal app window.
    #[allow(non_upper_case_globals)]
    static kCGWindowLayer: *const c_void;
}

// CoreFoundation collection/number FFI (linked transitively via CoreGraphics)
extern "C" {
    fn CFArrayGetCount(theArray: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(theArray: *const c_void, idx: isize) -> *const c_void;
    fn CFDictionaryGetValue(theDict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, theType: u32, valuePtr: *mut c_void) -> bool;
}

/// Known Japanese IME process names whose candidate (overlay) windows
/// indicate active composition.
const IME_PROCESS_NAMES: &[&str] = &[
    "JapaneseIM",          // Apple built-in Japanese IME
    "GoogleJapaneseInput", // Google Japanese Input
];

// ---------------------------------------------------------------------------
// ImeMonitor — cross-process IME composition (candidate window) detection
// ---------------------------------------------------------------------------

/// Monitors whether an IME candidate window is currently visible.
///
/// macOS: enumerates on-screen windows via `CGWindowListCopyWindowInfo` and
/// checks if any belong to a known Japanese IME process at a non-zero window
/// layer. When detected, the HMM engine is paused to avoid contaminating
/// keystroke features with candidate-navigation keystrokes.
pub struct ImeMonitor;

impl ImeMonitor {
    pub fn new() -> Self {
        Self
    }

    /// Returns `true` if a Japanese IME candidate window is currently on screen.
    ///
    /// Performs an early exit if `IME_OPEN` is false (user is in alphanumeric
    /// mode — no candidate window possible), making the common case zero-cost.
    pub fn is_candidate_window_open(&self) -> bool {
        // Fast path: if user is not in Japanese input mode, skip enumeration.
        if !crate::input::hook::IME_OPEN.load(Ordering::Relaxed) {
            return false;
        }
        unsafe { check_candidate_window() }
    }
}

/// Enumerate on-screen windows and check for an IME candidate overlay.
///
/// # Safety
/// Calls CoreGraphics FFI functions. Safe to call from any thread.
unsafe fn check_candidate_window() -> bool {
    let window_list = CGWindowListCopyWindowInfo(
        K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
        K_CG_NULL_WINDOW_ID,
    );
    if window_list.is_null() {
        return false;
    }

    let count = CFArrayGetCount(window_list);
    let mut found = false;

    for i in 0..count {
        let dict = CFArrayGetValueAtIndex(window_list, i);
        if dict.is_null() {
            continue;
        }

        // ── Check window layer ──────────────────────────────────────────
        // Normal application windows are at layer 0.
        // IME candidate windows appear at layer > 0.
        let layer_ptr = CFDictionaryGetValue(dict, kCGWindowLayer);
        if layer_ptr.is_null() {
            continue;
        }
        let mut layer: i32 = 0;
        let ok = CFNumberGetValue(
            layer_ptr,
            K_CF_NUMBER_SINT32_TYPE,
            &mut layer as *mut i32 as *mut c_void,
        );
        if !ok || layer <= 0 {
            continue;
        }

        // ── Check window owner name ─────────────────────────────────────
        let owner_ptr = CFDictionaryGetValue(dict, kCGWindowOwnerName);
        if owner_ptr.is_null() {
            continue;
        }
        // wrap_under_get_rule: dictionary owns the string; we must not release it.
        let owner_cf =
            CFString::wrap_under_get_rule(owner_ptr as core_foundation::string::CFStringRef);
        let owner_str = owner_cf.to_string();

        if IME_PROCESS_NAMES.iter().any(|&name| owner_str == name) {
            found = true;
            break;
        }
    }

    // Release the array returned by CGWindowListCopyWindowInfo (Copy rule).
    CFRelease(window_list);
    found
}

// ---------------------------------------------------------------------------
// spawn_ime_open_polling_thread — IME open/close state machine
//
// Detection strategy (two paths):
//
// Path A — JIS physical keys (fast, ~5ms):
//   CGEventTap in hook_macos.rs sets IME_OPEN + IME_STATE_DIRTY on
//   kVK_JIS_Eisu / kVK_JIS_Kana press. We trust this directly and skip TIS.
//   Reason: TIS returns the GSE *process-own* input source (always ABC),
//   not the foreground app's source, because macOS uses per-app input source
//   switching by default. CGEventTap is system-wide, so IME_OPEN is correct.
//
// Path B — ANSI/US keyboards (100ms TIS poll):
//   When no JIS key has ever been observed (JIS_KEYBOARD_SEEN = false),
//   fall back to TIS to detect menu-bar switches on ANSI keyboards.
//
//   IMPORTANT: Once any JIS key is observed (JIS_KEYBOARD_SEEN = true),
//   Path B is permanently disabled. TIS always returns GSE's own source (ABC)
//   on per-app input source systems (macOS default), so using TIS after a JIS
//   key press would immediately reset IME_OPEN to false — masking the correct
//   state set by Path A. On JIS keyboards we rely exclusively on IME_OPEN.
// ---------------------------------------------------------------------------
pub fn spawn_ime_open_polling_thread(
    log_tx: Sender<LogEntry>,
    wake_rx: crossbeam_channel::Receiver<()>,
) {
    std::thread::spawn(move || {
        let mut last_state: Option<bool> = None;

        loop {
            // Wait for keystroke wake signal, or fall back to 100ms idle poll.
            let woke_by_key = wake_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_ok();

            if woke_by_key {
                // Brief delay: lets the OS process the keypress through the IME engine.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }

            // ── Path A: JIS key was detected by CGEventTap ────────────────────
            // swap(false) atomically drains the dirty flag.
            let dirty = crate::input::hook::IME_STATE_DIRTY.swap(false, Ordering::AcqRel);

            let current_state = if dirty {
                // ── Path A: JIS key was detected by CGEventTap ────────────────────
                // Trust IME_OPEN set by hook callback — system-wide JIS key evidence.
                crate::input::hook::IME_OPEN.load(Ordering::Acquire)
            } else if crate::input::hook::JIS_KEYBOARD_SEEN.load(Ordering::Acquire) {
                // ── JIS keyboard mode: skip TIS, trust hook state ─────────────────
                // TIS returns GSE's own source (always ABC) on per-app systems,
                // which would immediately undo Path A's correct state.
                crate::input::hook::IME_OPEN.load(Ordering::Acquire)
            } else {
                // ── Path B: ANSI keyboard — TIS poll (menu-bar switch fallback) ───
                ime_macos::is_japanese_ime_open()
            };

            if last_state.map_or(true, |prev| prev != current_state) {
                last_state = Some(current_state);
                crate::input::hook::IME_OPEN.store(current_state, Ordering::Relaxed);
                emit_ime_state(&log_tx, current_state);
            }
        }
    });
}

/// Emit a `LogEntry::ImeState` with the current wall-clock timestamp.
fn emit_ime_state(log_tx: &Sender<LogEntry>, on: bool) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let _ = log_tx.try_send(LogEntry::ImeState { timestamp: ts, on });
    tracing::debug!(
        "IME open: {} (polling)",
        if on { "ON (あ)" } else { "OFF (A)" }
    );
}
