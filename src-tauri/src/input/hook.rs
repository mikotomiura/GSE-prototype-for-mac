use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crossbeam_channel::Sender;
use lazy_static::lazy_static;

use crate::analysis::features::InputEvent;

// ---------------------------------------------------------------------------
// Japanese IME mode VK codes — shared between platforms.
// On macOS: detected via JIS HID keycodes in CGEventTap callback.
// ---------------------------------------------------------------------------
pub const VK_DBE_ALPHANUMERIC: u32 = 0xF0; // 英数: Switch to alphanumeric (A mode)
pub const VK_DBE_KATAKANA: u32 = 0xF1; // カタカナ: Switch to katakana
pub const VK_DBE_HIRAGANA: u32 = 0xF2; // ひらがな: Switch to hiragana (あ mode)
pub const VK_DBE_SBCSCHAR: u32 = 0xF3; // 半角: Single-byte character mode
pub const VK_DBE_DBCSCHAR: u32 = 0xF4; // 全角: Double-byte character mode
pub const VK_KANJI: u32 = 0x19; // 半角/全角: Half-width / full-width toggle

/// Cross-process IME composition state (romaji→hiragana conversion in progress).
/// macOS: Always false (composition detection not implemented; known limitation).
pub static IME_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Returns true if an IME is currently composing text in the foreground application.
pub fn is_ime_active() -> bool {
    IME_ACTIVE.load(Ordering::Relaxed)
}

/// Japanese IME input mode: true = あ/カ (Japanese), false = A (alphanumeric/coding).
///
/// Updated by two mechanisms (in priority order):
///   1. VK_DBE_* / VK_KANJI key detection in CGEventTap callback — reliable, instant.
///   2. TIS polling via dispatch_sync_f to main queue — 100ms secondary.
///
/// The analysis thread reads this for HMM β-selection (writing vs coding baselines).
pub static IME_OPEN: AtomicBool = AtomicBool::new(false);

/// Returns true if the Japanese IME is currently in Japanese-input mode.
pub fn is_ime_open() -> bool {
    IME_OPEN.load(Ordering::Relaxed)
}

/// Dirty flag: set by hook callback when a VK_DBE_* / VK_KANJI key is pressed.
/// The IME open polling thread reads this to emit LogEntry::ImeState with a fresh timestamp.
pub static IME_STATE_DIRTY: AtomicBool = AtomicBool::new(false);

/// Set to true once any JIS IME key (VK_KANJI or VK_DBE_*) has been observed.
///
/// macOS uses per-app input source switching by default, which means
/// TISCopyCurrentKeyboardInputSource() always returns GSE's own source (ABC),
/// NOT the foreground app's source.  Once we see a physical JIS key we know
/// the user has a JIS keyboard, so we disable TIS polling and trust IME_OPEN
/// (managed by the hook callback) as the single source of truth.
pub static JIS_KEYBOARD_SEEN: AtomicBool = AtomicBool::new(false);

lazy_static! {
    pub static ref EVENT_SENDER: Mutex<Option<Sender<InputEvent>>> = Mutex::new(None);
    /// Wake channel for the IME open polling thread.
    /// Signalled on every keypress so the polling thread updates within ~5ms.
    pub static ref POLL_WAKE_TX: Mutex<Option<Sender<()>>> = Mutex::new(None);
}

/// Store the polling thread's wake channel sender.
/// Must be called before `init_hook` so the sender is ready when the hook fires.
pub fn set_poll_wake_sender(tx: Sender<()>) {
    let mut s = POLL_WAKE_TX.lock().unwrap();
    *s = Some(tx);
}

// ---------------------------------------------------------------------------
// macOS keyboard hook via CGEventTap
// ---------------------------------------------------------------------------

#[path = "hook_macos.rs"]
pub mod hook_macos;

/// Install the CGEventTap and begin delivering InputEvents to `sender`.
/// Spawns a dedicated background thread with its own CFRunLoop; returns immediately.
pub fn init_hook(sender: Sender<InputEvent>) {
    // Store sender in global state for use in hook callback.
    {
        let mut s = EVENT_SENDER.lock().unwrap();
        *s = Some(sender);
    }

    hook_macos::start();
}
