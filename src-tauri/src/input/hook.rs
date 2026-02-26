use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// Japanese IME mode VK codes — captured by WH_KEYBOARD_LL before IME engine.
//
// ImmGetContext() returns NULL for windows owned by other processes, making
// polling-only approaches unreliable for cross-process IME mode detection.
// These VK codes are the ONLY reliable cross-process IME mode indicator.
// They reach WH_KEYBOARD_LL before the IME engine processes them, and we
// perform only cheap atomic stores here (no ImmGet* calls → no deadlock risk).
// ---------------------------------------------------------------------------
const VK_DBE_ALPHANUMERIC: u32 = 0xF0; // 英数: Switch to alphanumeric (A mode)
const VK_DBE_KATAKANA: u32 = 0xF1; // カタカナ: Switch to katakana
const VK_DBE_HIRAGANA: u32 = 0xF2; // ひらがな: Switch to hiragana (あ mode)
const VK_DBE_SBCSCHAR: u32 = 0xF3; // 半角: Single-byte character mode
const VK_DBE_DBCSCHAR: u32 = 0xF4; // 全角: Double-byte character mode
const VK_KANJI: u32 = 0x19; // 半角/全角: Half-width / full-width toggle

use crossbeam_channel::Sender;
use lazy_static::lazy_static;
use windows::Win32::Foundation::{HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, HHOOK, KBDLLHOOKSTRUCT, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::analysis::features::InputEvent;

// IME WinEvent constants (winuser.h, Windows 8+)
// These fire in ANY foreground process - no DLL injection needed with WINEVENT_OUTOFCONTEXT
const EVENT_OBJECT_IME_CHANGE: u32 = 0x8016; // Composition string changed (romaji → hiragana)
const EVENT_OBJECT_IME_SHOW: u32 = 0x8017; // IME UI shown (candidate list appeared)
const EVENT_OBJECT_IME_HIDE: u32 = 0x8018; // IME UI hidden (composition confirmed/cancelled)

// WINEVENT flags (winuser.h) - not exposed as typed constants in windows crate 0.58
const WINEVENT_OUTOFCONTEXT: u32 = 0x0000; // Callback fires in hook thread (no DLL injection)
const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002; // Ignore IME events from our own process

/// Cross-process IME composition state (romaji→hiragana conversion in progress).
/// Updated by the WinEvent hook when IME starts/stops in ANY foreground application.
pub static IME_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Returns true if an IME is currently composing text in the foreground application.
pub fn is_ime_active() -> bool {
    IME_ACTIVE.load(Ordering::Relaxed)
}

/// Japanese IME input mode: true = あ/カ (Japanese), false = A (alphanumeric/coding).
///
/// Updated by two mechanisms (in priority order):
///   1. VK_DBE_* / VK_KANJI key detection in `hook_callback` — reliable, instant,
///      cross-process. Only atomic stores; no ImmGet* calls; no deadlock risk.
///   2. `ImmGetOpenStatus` polling in `spawn_ime_open_polling_thread` — cross-process
///      may return NULL on Windows 10/11; used as a secondary supplement only.
///
/// The analysis thread reads this for HMM β-selection (writing vs coding baselines).
pub static IME_OPEN: AtomicBool = AtomicBool::new(false);

/// Returns true if the Japanese IME is currently in Japanese-input mode.
pub fn is_ime_open() -> bool {
    IME_OPEN.load(Ordering::Relaxed)
}

/// Dirty flag: set by `hook_callback` when a VK_DBE_* / VK_KANJI key is pressed.
///
/// The IME open polling thread in `ime.rs` reads this flag and, when set, immediately
/// emits a `LogEntry::ImeState` entry. This decouples the log timestamp from the
/// keystroke timestamp, preventing same-millisecond flapping.
///
/// Uses `AcqRel` / `Acquire` ordering so the `IME_OPEN` write is visible to the
/// polling thread before it reads the dirty flag.
pub static IME_STATE_DIRTY: AtomicBool = AtomicBool::new(false);

// Wrapper to make HHOOK Send+Sync for lazy_static
struct ThreadSafeHook(#[allow(dead_code)] HHOOK);
unsafe impl Send for ThreadSafeHook {}
unsafe impl Sync for ThreadSafeHook {}

// Wrapper to make HWINEVENTHOOK Send+Sync for lazy_static
// The hook handle must stay alive for the duration of the app to keep the hook active.
struct ThreadSafeWinEventHook(#[allow(dead_code)] HWINEVENTHOOK);
unsafe impl Send for ThreadSafeWinEventHook {}
unsafe impl Sync for ThreadSafeWinEventHook {}

lazy_static! {
    static ref HOOK_HANDLE: Mutex<Option<ThreadSafeHook>> = Mutex::new(None);
    static ref WINEVENT_HOOK_HANDLE: Mutex<Option<ThreadSafeWinEventHook>> = Mutex::new(None);
    static ref EVENT_SENDER: Mutex<Option<Sender<InputEvent>>> = Mutex::new(None);
    /// Wake channel for the IME open polling thread.
    /// Signalled on every keypress so the polling thread can update IME state
    /// within ~5ms rather than waiting for the next 100ms polling cycle.
    static ref POLL_WAKE_TX: Mutex<Option<Sender<()>>> = Mutex::new(None);
}

/// Store the polling thread's wake channel sender.
/// Must be called before `init_hook` so the sender is ready when the hook fires.
pub fn set_poll_wake_sender(tx: Sender<()>) {
    let mut s = POLL_WAKE_TX.lock().unwrap();
    *s = Some(tx);
}

/// WinEvent callback for cross-process IME composition detection.
/// Called in the hook thread's message loop when IME events occur in any process.
/// WINEVENT_OUTOFCONTEXT ensures this runs in our thread without DLL injection.
unsafe extern "system" fn win_event_callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    match event {
        // Composition started or changed: romaji typed / hiragana displayed / candidate shown.
        //
        // Key insight: IME composition can ONLY start when the IME is in Japanese
        // input mode (あ/カ). Therefore this event is an authoritative cross-process
        // signal that IME_OPEN must be true — even when VK_DBE_HIRAGANA key-DOWN
        // never reached WH_KEYBOARD_LL (as happens on Surface Type Cover).
        EVENT_OBJECT_IME_SHOW | EVENT_OBJECT_IME_CHANGE => {
            IME_ACTIVE.store(true, Ordering::Relaxed);
            // Infer Japanese mode from composition start (cross-process, no VK needed).
            IME_OPEN.store(true, Ordering::Release);
            IME_STATE_DIRTY.store(true, Ordering::Release);
            // Wake the polling thread immediately so the ImeState log entry is
            // emitted within ~5ms of the composition start event.
            if let Ok(guard) = (*POLL_WAKE_TX).try_lock() {
                if let Some(tx) = guard.as_ref() {
                    let _ = tx.try_send(());
                }
            }
        }
        // Composition ended: user pressed Enter (confirm) or Escape (cancel)
        EVENT_OBJECT_IME_HIDE => {
            IME_ACTIVE.store(false, Ordering::Relaxed);
        }
        _ => {}
    }
}

pub fn init_hook(sender: Sender<InputEvent>) {
    // Store sender in global state for use in hook callback
    {
        let mut s = (*EVENT_SENDER).lock().unwrap();
        *s = Some(sender);
    }

    // Spawn dedicated hook thread
    // Both WH_KEYBOARD_LL and WinEvent hooks require a message loop to function.
    thread::spawn(|| {
        unsafe {
            // --- Step 1: Install low-level keyboard hook ---
            let hook_id =
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_callback), HINSTANCE::default(), 0);

            match hook_id {
                Ok(h) => {
                    {
                        let mut handle = (*HOOK_HANDLE).lock().unwrap();
                        *handle = Some(ThreadSafeHook(h));
                    }
                    tracing::info!("Keyboard hook installed");
                }
                Err(e) => {
                    tracing::error!("Failed to install keyboard hook: {:?}", e);
                    return;
                }
            }

            // --- Step 2: Install WinEvent hook for cross-process IME composition ---
            // Range [EVENT_OBJECT_IME_CHANGE=0x8016, EVENT_OBJECT_IME_HIDE=0x8018] covers
            // the entire IME lifecycle: composition start → candidate display → commit/cancel.
            //
            // WINEVENT_OUTOFCONTEXT: callback fires in our thread's message loop.
            //   - No DLL injection required.
            //   - Works across all processes including UWP/sandboxed apps.
            // WINEVENT_SKIPOWNPROCESS: ignore IME events from our own process.
            let ime_hook = SetWinEventHook(
                EVENT_OBJECT_IME_CHANGE, // 0x8016: fires when romaji → hiragana conversion starts
                EVENT_OBJECT_IME_HIDE,   // 0x8018: fires when composition ends
                HMODULE::default(), // NULL - no DLL needed for WINEVENT_OUTOFCONTEXT
                Some(win_event_callback),
                0, // All processes
                0, // All threads
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );

            if ime_hook.is_invalid() {
                tracing::warn!("Failed to install IME WinEvent hook");
            } else {
                tracing::info!("IME WinEvent hook installed (cross-process IME detection active)");
                let mut handle = (*WINEVENT_HOOK_HANDLE).lock().unwrap();
                *handle = Some(ThreadSafeWinEventHook(ime_hook));
            }

            // --- Step 3: Message loop ---
            // Required for WH_KEYBOARD_LL to receive events.
            // Also required for WinEvent callbacks (WINEVENT_OUTOFCONTEXT) to be delivered:
            // With WINEVENT_OUTOFCONTEXT, Windows posts events to the thread's message queue
            // and dispatches them via DispatchMessageW. Both calls are required.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn hook_callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event_type = wparam.0 as u32;
        let is_press = event_type == WM_KEYDOWN || event_type == WM_SYSKEYDOWN;
        let is_release = event_type == WM_KEYUP || event_type == WM_SYSKEYUP;

        if is_press || is_release {
            let vk_code = (*(lparam.0 as *const KBDLLHOOKSTRUCT)).vkCode;
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            // ---------------------------------------------------------------------------
            // IME mode tracking via VK_DBE_* keys.
            //
            // WHY here and not in the polling thread:
            //   ImmGetContext() returns NULL for windows owned by other processes on
            //   Windows 10/11, making ImmGetOpenStatus unreliable cross-process.
            //   VK_DBE_* keys reach WH_KEYBOARD_LL reliably before the IME engine,
            //   making this the ONLY dependable cross-process mode-switch detector.
            //
            // WHY both key-DOWN and key-UP are handled:
            //   Surface Type Cover sends VK_DBE_HIRAGANA (0xF2) ONLY as key-UP.
            //   VK_DBE_ALPHANUMERIC (0xF0) is sent ONLY as key-DOWN, paired with
            //   the ひらがな key-UP at the same millisecond. Both events must be
            //   handled to cover this asymmetric keyboard behavior.
            //
            // WHY no flapping:
            //   We only set IME_OPEN (atomic store, no ImmGet* calls) and set
            //   IME_STATE_DIRTY to wake the polling thread. The polling thread emits
            //   LogEntry::ImeState at its own timestamp (not the key's timestamp),
            //   so the log entry is always temporally separated from the key event.
            // ---------------------------------------------------------------------------
            if is_press {
                match vk_code {
                    VK_DBE_ALPHANUMERIC | VK_DBE_SBCSCHAR => {
                        IME_OPEN.store(false, Ordering::Release);
                        IME_STATE_DIRTY.store(true, Ordering::Release);
                    }
                    VK_DBE_KATAKANA | VK_DBE_HIRAGANA | VK_DBE_DBCSCHAR => {
                        IME_OPEN.store(true, Ordering::Release);
                        IME_STATE_DIRTY.store(true, Ordering::Release);
                    }
                    VK_KANJI => {
                        let current = IME_OPEN.load(Ordering::Relaxed);
                        IME_OPEN.store(!current, Ordering::Release);
                        IME_STATE_DIRTY.store(true, Ordering::Release);
                    }
                    _ => {}
                }
            }

            // Surface Type Cover: VK_DBE_HIRAGANA fires ONLY as key-UP on this device.
            // Mirror the press handler on key-up to cover asymmetric keyboards.
            // Duplicate stores are harmless — the polling thread deduplicates via last_state.
            if is_release {
                match vk_code {
                    VK_DBE_KATAKANA | VK_DBE_HIRAGANA | VK_DBE_DBCSCHAR => {
                        IME_OPEN.store(true, Ordering::Release);
                        IME_STATE_DIRTY.store(true, Ordering::Release);
                    }
                    VK_DBE_ALPHANUMERIC | VK_DBE_SBCSCHAR => {
                        IME_OPEN.store(false, Ordering::Release);
                        IME_STATE_DIRTY.store(true, Ordering::Release);
                    }
                    _ => {}
                }
            }

            let event = InputEvent {
                vk_code,
                timestamp,
                is_press,
            };

            // Non-blocking send of keystroke event to analysis thread
            if let Ok(guard) = (*EVENT_SENDER).try_lock() {
                if let Some(sender) = guard.as_ref() {
                    let _ = sender.try_send(event);
                }
            }

            // Wake the IME polling thread on every keypress, and also on VK_DBE_* key-up.
            // Surface Type Cover sends VK_DBE_HIRAGANA (0xF2) ONLY as key-UP, so
            // we must send the wake signal on is_release for IME mode keys too.
            // Non-blocking: drops silently if channel is full (already woken).
            let is_ime_mode_key = matches!(
                vk_code,
                VK_DBE_ALPHANUMERIC
                    | VK_DBE_KATAKANA
                    | VK_DBE_HIRAGANA
                    | VK_DBE_SBCSCHAR
                    | VK_DBE_DBCSCHAR
                    | VK_KANJI
            );
            if is_press || (is_release && is_ime_mode_key) {
                if let Ok(guard) = (*POLL_WAKE_TX).try_lock() {
                    if let Some(tx) = guard.as_ref() {
                        let _ = tx.try_send(());
                    }
                }
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}
