use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;

use crate::logger::LogEntry;

// ---------------------------------------------------------------------------
// Platform-specific IME monitor implementations
// ---------------------------------------------------------------------------

#[path = "ime_macos.rs"]
pub mod ime_macos;

// ---------------------------------------------------------------------------
// ImeMonitor — cross-process IME composition (candidate window) detection
// ---------------------------------------------------------------------------

/// Monitors whether an IME candidate window is currently open.
///
/// macOS: stub returning false (IME_ACTIVE composition detection not implemented).
///        HMM continues running during candidate selection — known limitation.
pub struct ImeMonitor {}

impl ImeMonitor {
    pub fn new() -> Self {
        Self {}
    }

    pub fn is_candidate_window_open(&self) -> bool {
        false
    }
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
