use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use windows::{
    core::*,
    Win32::Foundation::{BOOL, HWND, LPARAM},
    Win32::System::Com::*,
    Win32::UI::Accessibility::*,
    Win32::UI::Input::Ime::{ImmGetContext, ImmGetOpenStatus, ImmReleaseContext},
    Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetForegroundWindow, IsWindowVisible},
};

use crate::logger::LogEntry;

pub struct ImeMonitor {
    automation: Option<IUIAutomation>,
}

impl ImeMonitor {
    pub fn new() -> Self {
        unsafe {
            // COM is initialized here for the UIAutomation fallback.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            let automation: Result<IUIAutomation> =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER);

            if let Ok(auto) = automation {
                Self {
                    automation: Some(auto),
                }
            } else {
                Self { automation: None }
            }
        }
    }

    /// Returns true if IME (Japanese input method, etc.) is currently composing text.
    ///
    /// Detection strategy (in priority order):
    ///
    /// 1. **WinEvent (PRIMARY)**: The WinEvent hook in hook.rs monitors
    ///    EVENT_OBJECT_IME_CHANGE/SHOW/HIDE across all processes via WINEVENT_OUTOFCONTEXT.
    ///    This is the only method that reliably detects the romaji→hiragana composition
    ///    phase (before the candidate list appears). Works for any foreground application.
    ///
    ///    WHY the old ImmGetContext approach failed:
    ///    ImmGetContext() returns NULL for windows owned by a different process.
    ///    Since we never type in our own app, it ALWAYS returned NULL.
    ///
    /// 2. **EnumWindows (SECONDARY)**: Scans all top-level windows for visible IME
    ///    candidate windows by class name. Covers the candidate-selection phase as a
    ///    belt-and-suspenders fallback if the WinEvent hasn't fired yet.
    ///
    /// 3. **UIAutomation (TERTIARY)**: Original approach retained as last resort.
    ///    Limited: only fires when the focused element is the IME window itself,
    ///    which doesn't happen during normal composition.
    pub fn is_candidate_window_open(&self) -> bool {
        let winevent_active = crate::input::hook::is_ime_active();

        // --- SECONDARY: EnumWindows scan for visible IME candidate windows ---
        // NOTE: "MSCTFIME UI" is intentionally excluded here.
        // That class belongs to the language bar (A/あ button on taskbar) which is
        // ALWAYS visible when Japanese IME is loaded, even when NOT composing.
        // Including it caused a permanent false-positive that paused all analysis.
        let enumwindows_active = is_ime_candidate_window_visible();

        // --- TERTIARY: UIAutomation focused-element check ---
        let uia_active = if let Some(auto) = &self.automation {
            let mut found = false;
            unsafe {
                if let Ok(element) = auto.GetFocusedElement() {
                    if let Ok(class_name) = element.CurrentClassName() {
                        let name = class_name.to_string();
                        if name.contains("Candidate") || name.contains("Ime") {
                            found = true;
                        }
                    }
                    if !found {
                        if let Ok(name_bstr) = element.CurrentName() {
                            let name = name_bstr.to_string();
                            if name.contains("候補") {
                                found = true;
                            }
                        }
                    }
                }
            }
            found
        } else {
            false
        };

        // Safety: if WinEvent says IME is active but neither EnumWindows nor UIAutomation
        // confirms a visible candidate window, the IME_ACTIVE flag is stale.
        // This happens when EVENT_OBJECT_IME_HIDE was missed (e.g. focus change, crash).
        // Reset it here to prevent permanently pausing all keystroke analysis.
        if winevent_active && !enumwindows_active && !uia_active {
            crate::input::hook::IME_ACTIVE.store(false, Ordering::Relaxed);
            return false;
        }

        winevent_active || enumwindows_active || uia_active
    }
}

/// Scan all top-level windows for visible IME candidate windows using EnumWindows.
///
/// Known IME candidate window class names on Windows 10/11:
///   - "CandidateUI_UIElement" : Modern IME candidate list (Microsoft IME, Google IME)
///   - "IME"               : Classic IMM32 IME window
///   - "*Candidate*"       : Catch-all for other IME implementations
///
/// INTENTIONALLY EXCLUDED:
///   - "MSCTFIME UI" : This is the TSF language bar (A/あ indicator on the taskbar).
///     It is ALWAYS visible on systems with Japanese IME loaded, even during normal
///     Latin text entry. Including it caused the engine to be permanently paused.
fn is_ime_candidate_window_visible() -> bool {
    // The callback sets *lparam to true and stops enumeration when an IME window is found.
    unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // Skip invisible windows
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1); // Continue enumeration
        }

        let mut buf = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut buf);
        if len > 0 {
            let class = String::from_utf16_lossy(&buf[..len as usize]);

            // Match visible IME candidate windows only.
            // Do NOT include "MSCTFIME" - that is the always-visible language bar.
            if class.contains("CandidateUI")
                || class == "IME"
                || class.contains("Candidate")
            {
                // Signal found via lparam (pointer to bool on caller's stack)
                *(lparam.0 as *mut bool) = true;
                return BOOL(0); // Stop enumeration
            }
        }

        BOOL(1) // Continue enumeration
    }

    let mut found = false;
    unsafe {
        // Ignore the Result: BOOL(0) from our callback is interpreted as "error"
        // by EnumWindows, but it just means we stopped early after finding a match.
        let _ = EnumWindows(Some(callback), LPARAM(&mut found as *mut bool as isize));
    }
    found
}

// Kept for API compatibility (unused)
pub fn is_ime_active_check() -> bool {
    false
}

/// Spawn a dedicated background thread for IME open/close state tracking.
///
/// # Detection strategy (layered, in priority order)
///
/// **Primary — `ImmGetOpenStatus` polling:**
/// When `ImmGetContext(foreground_hwnd)` returns a non-null HIMC, `ImmGetOpenStatus`
/// gives the authoritative IME mode. This works when the foreground window is owned
/// by a process whose IME context is accessible (rare on Windows 10/11 for cross-process
/// windows, but used as a supplement when available).
///
/// **Fallback — `IME_STATE_DIRTY` flag from VK_DBE_* keys:**
/// `hook_callback` detects VK_DBE_* / VK_KANJI key presses and atomically updates
/// `IME_OPEN` + sets `IME_STATE_DIRTY`. When `ImmGetContext` returns NULL (typical for
/// cross-process windows), this thread reads the dirty flag and emits `LogEntry::ImeState`
/// at its own timestamp — ensuring the log entry is always temporally separated from
/// the triggering keystroke (no same-millisecond flapping).
///
/// # Lag reduction (ADVICE.md v2 requirement)
///
/// Instead of a fixed 100ms sleep, the thread waits on `wake_rx` with a 100ms timeout.
/// `hook_callback` sends a wake signal on every keypress (non-blocking `try_send`).
/// On wake, we sleep 5ms to let the OS route the key through the IME engine, then poll.
/// Maximum lag during active typing: ~5ms (down from 100ms).
///
/// # No flapping guarantee
///
/// `LogEntry::ImeState` is emitted only from THIS thread, never from the analysis thread
/// processing the key event. The timestamp is `SystemTime::now()` at the moment of
/// emission — always ≥5ms after the triggering keystroke.
pub fn spawn_ime_open_polling_thread(
    log_tx: Sender<LogEntry>,
    wake_rx: crossbeam_channel::Receiver<()>,
) {
    std::thread::spawn(move || {
        // Option<bool>: None = まだ一度も検出していない初期状態。
        // Some(v) = 前回 emit した状態。
        // None の場合は current_state が false でも必ず emit する
        // (起動直後に IME が A モードのまま VK 240 を押した場合を捕捉する)。
        let mut last_state: Option<bool> = None;

        loop {
            // Wait for a keystroke wake signal, or fall back to 100ms idle poll.
            let woke_by_key = wake_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_ok();

            if woke_by_key {
                // Brief delay: gives the OS time to route the key through the IME
                // engine so that ImmGetOpenStatus / IME_OPEN reflect the new mode.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }

            // --- PRIMARY: ImmGetContext-based detection ---
            // May return NULL for cross-process windows on Windows 10/11.
            let mut imm_succeeded = false;
            unsafe {
                let hwnd = GetForegroundWindow();
                if !hwnd.0.is_null() {
                    let himc = ImmGetContext(hwnd);
                    if !himc.0.is_null() {
                        let current_state = ImmGetOpenStatus(himc).as_bool();
                        let _ = ImmReleaseContext(hwnd, himc);
                        imm_succeeded = true;

                        // last_state が None (初回) か、前回と異なる場合のみ emit
                        if last_state.map_or(true, |prev| prev != current_state) {
                            last_state = Some(current_state);
                            crate::input::hook::IME_OPEN.store(current_state, Ordering::Relaxed);
                            emit_ime_state(&log_tx, current_state);
                        }
                    }
                }
            }

            // --- FALLBACK: VK_DBE_* dirty flag from hook_callback ---
            // Used when ImmGetContext returns NULL (typical cross-process case).
            // IME_OPEN was already updated by the hook; we just need to detect the
            // change and emit the log entry with a fresh timestamp.
            if !imm_succeeded {
                if crate::input::hook::IME_STATE_DIRTY
                    .swap(false, Ordering::AcqRel)
                {
                    let current_state =
                        crate::input::hook::IME_OPEN.load(Ordering::Acquire);
                    // last_state が None (初回) か、前回と異なる場合のみ emit
                    if last_state.map_or(true, |prev| prev != current_state) {
                        last_state = Some(current_state);
                        emit_ime_state(&log_tx, current_state);
                    }
                }
            }
        }
    });
}

/// Emit a `LogEntry::ImeState` to the logger channel with the current wall-clock time.
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
