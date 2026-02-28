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
// - recv_timeout(100ms) idle poll with wake-on-keystroke optimization
// - last_state deduplication (emit only on change)
// - Detection: TIS TISCopyCurrentKeyboardInputSource + "inputmethod.Japanese" check
//   (runs on main GCD queue via dispatch_sync_f)
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
                // Brief delay: lets the OS process the keypress through the IME engine
                // so that platform state (ImmGetOpenStatus / TIS) reflects the new mode.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }

            // ── macOS primary: TIS input source check ─────────────────────────
            let current_state = ime_macos::is_japanese_ime_open();

            if last_state.map_or(true, |prev| prev != current_state) {
                last_state = Some(current_state);
                crate::input::hook::IME_OPEN.store(current_state, Ordering::Relaxed);
                emit_ime_state(&log_tx, current_state);
            }

            // Drain VK_DBE_* dirty flag set by hook callback for JIS keys.
            // TIS always succeeds, so this is just housekeeping.
            crate::input::hook::IME_STATE_DIRTY.store(false, Ordering::Relaxed);
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
