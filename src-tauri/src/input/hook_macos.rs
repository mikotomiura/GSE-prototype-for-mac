// macOS keyboard hook via CGEventTap.
// Uses kCGEventTapOptionListenOnly → requires Input Monitoring permission only.
// Runs on a dedicated thread with its own CFRunLoop, mirroring the Windows WH_KEYBOARD_LL design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use core_foundation::runloop::{kCFRunLoopDefaultMode, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    EventField,
};

use crate::analysis::features::{InputEvent, is_typing_key};
use super::{
    EVENT_SENDER, IME_ACTIVE, IME_COMPOSING, IME_OPEN, IME_STATE_DIRTY,
    JIS_KEYBOARD_SEEN, LAST_KEYSTROKE_TIMESTAMP, POLL_WAKE_TX,
    VK_DBE_ALPHANUMERIC, VK_DBE_DBCSCHAR, VK_DBE_HIRAGANA, VK_DBE_KATAKANA,
    VK_DBE_SBCSCHAR, VK_KANJI,
};

// Input Monitoring permission check/request.
// These are in the ApplicationServices framework, umbrella-linked via CoreGraphics.
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

/// true when CGEventTap is successfully installed and receiving events.
/// Read by `get_hook_status` Tauri command to report permission state to the frontend.
pub static HOOK_ACTIVE: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// macOS CGKeyCode → Windows VK equivalent mapping
//
// `EventField::KEYBOARD_EVENT_KEYCODE` returns the macOS *virtual key code*
// (CGKeyCode / kVK_* constants from HIToolbox/Events.h), which is DIFFERENT
// from the USB HID keycode.  Notable differences:
//   kVK_ANSI_A = 0x00  (HID 'A' = 0x04)
//   kVK_Delete (Backspace) = 0x33  (HID Backspace = 0x2A)
//   kVK_ForwardDelete = 0x75      (HID Delete = 0x4C)
//
// GSE-critical mappings (affect F3, F6, backspace streak detection):
//   macOS 0x33 (Backspace)    → Windows 0x08 (VK_BACK)
//   macOS 0x75 (ForwardDel)   → Windows 0x2E (VK_DELETE)
// ---------------------------------------------------------------------------
fn macos_vk_to_vk(mac_vk: u64) -> u32 {
    match mac_vk {
        // ── Letters (kVK_ANSI_* → Windows VK_*) ─────────────────────────────
        0x00 => 0x41, // kVK_ANSI_A
        0x0B => 0x42, // kVK_ANSI_B
        0x08 => 0x43, // kVK_ANSI_C
        0x02 => 0x44, // kVK_ANSI_D
        0x0E => 0x45, // kVK_ANSI_E
        0x03 => 0x46, // kVK_ANSI_F
        0x05 => 0x47, // kVK_ANSI_G
        0x04 => 0x48, // kVK_ANSI_H
        0x22 => 0x49, // kVK_ANSI_I
        0x26 => 0x4A, // kVK_ANSI_J
        0x28 => 0x4B, // kVK_ANSI_K
        0x25 => 0x4C, // kVK_ANSI_L
        0x2E => 0x4D, // kVK_ANSI_M
        0x2D => 0x4E, // kVK_ANSI_N
        0x1F => 0x4F, // kVK_ANSI_O
        0x23 => 0x50, // kVK_ANSI_P
        0x0C => 0x51, // kVK_ANSI_Q
        0x0F => 0x52, // kVK_ANSI_R
        0x01 => 0x53, // kVK_ANSI_S
        0x11 => 0x54, // kVK_ANSI_T
        0x20 => 0x55, // kVK_ANSI_U
        0x09 => 0x56, // kVK_ANSI_V
        0x0D => 0x57, // kVK_ANSI_W
        0x07 => 0x58, // kVK_ANSI_X
        0x10 => 0x59, // kVK_ANSI_Y
        0x06 => 0x5A, // kVK_ANSI_Z

        // ── Digits ───────────────────────────────────────────────────────────
        0x12 => 0x31, // kVK_ANSI_1
        0x13 => 0x32, // kVK_ANSI_2
        0x14 => 0x33, // kVK_ANSI_3  (output 0x33 = VK_3, NOT Backspace)
        0x15 => 0x34, // kVK_ANSI_4
        0x17 => 0x35, // kVK_ANSI_5
        0x16 => 0x36, // kVK_ANSI_6
        0x1A => 0x37, // kVK_ANSI_7
        0x1C => 0x38, // kVK_ANSI_8
        0x19 => 0x39, // kVK_ANSI_9
        0x1D => 0x30, // kVK_ANSI_0

        // ── CRITICAL: editing keys for F3/F6/backspace-streak ─────────────
        0x33 => 0x08, // kVK_Delete (Backspace)  → VK_BACK
        0x75 => 0x2E, // kVK_ForwardDelete       → VK_DELETE

        // ── Control/whitespace ────────────────────────────────────────────
        0x24 => 0x0D, // kVK_Return  → VK_RETURN
        0x30 => 0x09, // kVK_Tab     → VK_TAB
        0x31 => 0x20, // kVK_Space   → VK_SPACE
        0x35 => 0x1B, // kVK_Escape  → VK_ESCAPE

        // ── Arrow keys ───────────────────────────────────────────────────
        0x7B => 0x25, // kVK_LeftArrow  → VK_LEFT
        0x7C => 0x27, // kVK_RightArrow → VK_RIGHT
        0x7D => 0x28, // kVK_DownArrow  → VK_DOWN
        0x7E => 0x26, // kVK_UpArrow    → VK_UP

        // ── Function keys ─────────────────────────────────────────────────
        0x7A => 0x70, // kVK_F1
        0x78 => 0x71, // kVK_F2
        0x63 => 0x72, // kVK_F3
        0x76 => 0x73, // kVK_F4
        0x60 => 0x74, // kVK_F5
        0x61 => 0x75, // kVK_F6
        0x62 => 0x76, // kVK_F7
        0x64 => 0x77, // kVK_F8
        0x65 => 0x78, // kVK_F9
        0x6D => 0x79, // kVK_F10
        0x67 => 0x7A, // kVK_F11
        0x6F => 0x7B, // kVK_F12

        // ── Symbols / punctuation ─────────────────────────────────────────
        0x18 => 0xBB, // kVK_ANSI_Equal          → VK_OEM_PLUS
        0x1B => 0xBD, // kVK_ANSI_Minus          → VK_OEM_MINUS
        0x21 => 0xDB, // kVK_ANSI_LeftBracket    → VK_OEM_4
        0x1E => 0xDD, // kVK_ANSI_RightBracket   → VK_OEM_6
        0x27 => 0xDE, // kVK_ANSI_Quote          → VK_OEM_7
        0x29 => 0xBA, // kVK_ANSI_Semicolon      → VK_OEM_1
        0x2A => 0xDC, // kVK_ANSI_Backslash      → VK_OEM_5
        0x2B => 0xBC, // kVK_ANSI_Comma          → VK_OEM_COMMA
        0x2C => 0xBF, // kVK_ANSI_Slash          → VK_OEM_2
        0x2F => 0xBE, // kVK_ANSI_Period         → VK_OEM_PERIOD
        0x32 => 0xC0, // kVK_ANSI_Grave          → VK_OEM_3

        // ── JIS IME toggle keys (CGKeyCode values for JIS keyboards) ──────
        0x66 => VK_DBE_ALPHANUMERIC, // kVK_JIS_Eisu → 英数 (alphanumeric)
        0x68 => VK_KANJI,            // kVK_JIS_Kana → かな toggle
        0x90 => VK_DBE_HIRAGANA,     // alternate かな on some JIS layouts
        0x91 => VK_DBE_KATAKANA,     // alternate カタカナ on some JIS layouts

        // Pass-through for unmapped keys.
        // Key identity is not critical for timing features (F1-F5).
        other => other as u32,
    }
}

// ---------------------------------------------------------------------------
// CGEventTap callback
// ---------------------------------------------------------------------------
fn handle_event(event_type: CGEventType, event: &CGEvent) {
    let is_press = matches!(event_type, CGEventType::KeyDown);
    let is_release = matches!(event_type, CGEventType::KeyUp);

    if !is_press && !is_release {
        return;
    }

    let mac_vk = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u64;
    let vk_code = macos_vk_to_vk(mac_vk);

    // macOS キーリピート検出: KeyDown のみ連射され KeyUp は最後の1回だけ。
    // リピートイベントは flight time 計算を狂わせるため InputEvent でマークする。
    let is_repeat = is_press
        && event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0;

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Update last keystroke timestamp for idle detection.
    // NOTE: 矢印キー等の非文字キーも LAST_KEYSTROKE_TIMESTAMP を更新するが、
    // HMM（分析チャネル）には送られずサイレンス扱いとなる。そのため
    // 「UI 上は Active だが HMM 推論上は Incubation（沈黙）」という
    // 意図的な状態乖離が発生し得る（非文字キーだけの操作は稀なため許容）。
    LAST_KEYSTROKE_TIMESTAMP.store(timestamp, Ordering::Relaxed);

    // IME mode tracking via JIS physical IME keys.
    // Mirrors the Windows VK_DBE_* handler in windows_impl.rs.
    // ANSI keyboards: these codes never arrive; TIS polling in ime_macos.rs
    // handles mode detection instead.
    if is_press {
        match vk_code {
            VK_DBE_ALPHANUMERIC | VK_DBE_SBCSCHAR => {
                JIS_KEYBOARD_SEEN.store(true, Ordering::Relaxed);
                IME_OPEN.store(false, Ordering::Release);
                IME_STATE_DIRTY.store(true, Ordering::Release);
            }
            VK_DBE_KATAKANA | VK_DBE_HIRAGANA | VK_DBE_DBCSCHAR => {
                JIS_KEYBOARD_SEEN.store(true, Ordering::Relaxed);
                IME_OPEN.store(true, Ordering::Release);
                IME_STATE_DIRTY.store(true, Ordering::Release);
            }
            VK_KANJI => {
                JIS_KEYBOARD_SEEN.store(true, Ordering::Relaxed);
                IME_OPEN.fetch_xor(true, Ordering::AcqRel);
                IME_STATE_DIRTY.store(true, Ordering::Release);
            }
            _ => {}
        }
    }
    // Also handle key-up for JIS IME keys (mirrors Surface Type Cover quirk handling)
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

    // ── IME composition / candidate state machine ────────────────────────
    // Tracks whether the user is navigating the IME candidate window.
    // Only active when IME_OPEN is true (Japanese input mode).
    //
    // State transitions (on keyDown only):
    //   Letter/digit → COMPOSING (inline text being composed)
    //   Space while COMPOSING → IME_ACTIVE (candidate window shown)
    //   Enter/Escape → reset both (composition confirmed/cancelled)
    //   Letter while IME_ACTIVE → reset IME_ACTIVE (candidate accepted, new composition)
    //   Backspace while IME_ACTIVE → reset IME_ACTIVE (back to editing inline)
    //   IME mode switch to off → reset both
    if is_press {
        if IME_OPEN.load(Ordering::Relaxed) {
            update_compose_state(vk_code);
        } else {
            // IME switched off → reset composition state.
            let was_composing = IME_COMPOSING.swap(false, Ordering::Release);
            let was_active = IME_ACTIVE.swap(false, Ordering::Release);
            if was_composing || was_active {
                tracing::debug!("IME compose state reset (IME_OPEN=false)");
            }
        }
    }

    // Send keystroke event to analysis thread (lock-free, non-blocking).
    // Filter at hook level: skip key repeats (cause Press/Release imbalance and
    // inflate backspace streak) and non-typing keys (arrows, function keys, IME
    // toggles — don't affect typing rhythm). IME tracking and
    // LAST_KEYSTROKE_TIMESTAMP are already handled above.
    //
    // NOTE: キーリピート除外により、Backspace 長押し（hold-to-delete）は Press 1回
    // + Release 1回のみ分析スレッドに届く。結果として:
    //   - register_keystroke() の BS ストリークは長押しでは増加しない（閾値14に未達）
    //   - F3（修正率）も長押し削除を1回としかカウントしない
    // これは「長押し = 意図的な一括削除 ≠ フラストレーション」という設計判断に基づく。
    // 連打（タップ連射）のみが Stuck シグナルとして検出される。
    if !is_repeat && is_typing_key(vk_code) {
        if let Some(sender) = EVENT_SENDER.get() {
            let _ = sender.try_send(InputEvent {
                vk_code,
                timestamp,
                is_press,
                is_repeat,
            });
        }
    }

    // Wake the IME polling thread on every keypress
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
        if let Some(tx) = POLL_WAKE_TX.get() {
            let _ = tx.try_send(());
        }
    }
}

// ---------------------------------------------------------------------------
// IME composition state machine
// ---------------------------------------------------------------------------

/// Update IME_COMPOSING and IME_ACTIVE based on the current keyDown VK code.
/// Called only when IME_OPEN is true and the event is a keyDown.
fn update_compose_state(vk: u32) {
    let composing = IME_COMPOSING.load(Ordering::Relaxed);
    let active = IME_ACTIVE.load(Ordering::Relaxed);

    // VK_SPACE = 0x20, VK_RETURN = 0x0D, VK_ESCAPE = 0x1B, VK_BACK = 0x08, VK_TAB = 0x09
    match vk {
        // Letter keys (A-Z: 0x41-0x5A) and digit keys (0-9: 0x30-0x39)
        // → Start/continue composition.
        0x30..=0x39 | 0x41..=0x5A => {
            if active {
                // Typing a new character while candidates showing →
                // accept current candidate, start new composition.
                IME_ACTIVE.store(false, Ordering::Release);
                tracing::debug!("IME_ACTIVE → false (new char during candidate)");
            }
            if !composing {
                IME_COMPOSING.store(true, Ordering::Release);
                tracing::debug!("IME_COMPOSING → true (char typed in Japanese mode)");
            }
        }
        // Space → trigger conversion (show candidate window) if composing.
        0x20 => {
            if composing && !active {
                IME_ACTIVE.store(true, Ordering::Release);
                tracing::debug!("IME_ACTIVE → true (Space during composition → candidates)");
            }
            // If already active, Space cycles to next candidate — stay active.
        }
        // Enter → confirm composition/candidate selection.
        0x0D => {
            if composing || active {
                IME_COMPOSING.store(false, Ordering::Release);
                IME_ACTIVE.store(false, Ordering::Release);
                tracing::debug!("IME compose/active → false (Enter confirms)");
            }
        }
        // Escape → cancel composition.
        0x1B => {
            if composing || active {
                IME_COMPOSING.store(false, Ordering::Release);
                IME_ACTIVE.store(false, Ordering::Release);
                tracing::debug!("IME compose/active → false (Escape cancels)");
            }
        }
        // Backspace while candidating → back to inline editing.
        0x08 => {
            if active {
                IME_ACTIVE.store(false, Ordering::Release);
                tracing::debug!("IME_ACTIVE → false (Backspace exits candidate)");
            }
            // Stay composing (editing inline text).
        }
        // Arrow keys (Left/Up/Right/Down: 0x25-0x28) → candidate navigation.
        // Stay in current state.
        0x25..=0x28 => {}
        // IME mode keys — handled elsewhere, ignore here.
        _ if vk >= 0xF0 || vk == VK_KANJI => {}
        // Other keys → reset composition state (e.g., Tab, function keys).
        _ => {
            if composing || active {
                IME_COMPOSING.store(false, Ordering::Release);
                IME_ACTIVE.store(false, Ordering::Release);
                tracing::debug!("IME compose/active → false (other key: VK=0x{:02X})", vk);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Install the CGEventTap and start listening on a dedicated thread with CFRunLoop.
///
/// Permission flow:
/// 1. Check `CGPreflightListenEventAccess()` — fast, no side-effects.
/// 2. If not granted, call `CGRequestListenEventAccess()` — opens System Settings
///    automatically to the Input Monitoring pane and returns false.
///    The user must grant access and restart the app.
/// 3. If granted, create CGEventTap and run CFRunLoop.
///
/// `HOOK_ACTIVE` is set to true only after a successful tap installation.
pub fn start() {
    thread::spawn(|| {
        // --- Step 1: Check current permission status ---
        let already_granted = unsafe { CGPreflightListenEventAccess() };

        if !already_granted {
            // --- Step 2: Request permission (opens System Settings) ---
            tracing::warn!(
                "Input Monitoring permission not granted. \
                 Opening System Settings > Privacy & Security > Input Monitoring..."
            );
            unsafe { CGRequestListenEventAccess() };
            // CGRequestListenEventAccess returns false and opens System Settings.
            // The app must be restarted after the user grants access.
            // HOOK_ACTIVE remains false — the frontend will show a permission banner.
            tracing::warn!(
                "Please grant Input Monitoring access to gse-next in System Settings, \
                 then restart the app."
            );
            return;
        }

        // --- Step 3: Permission granted — install CGEventTap ---
        //
        // Use TailAppendEventTap (not HeadInsertEventTap) with ListenOnly.
        // HeadInsert is designed for *active* taps that modify events; pairing it
        // with ListenOnly is unreliable on macOS 13+ and may silently drop events.
        // TailAppend is the correct placement for passive (listen-only) monitors.
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::TailAppendEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::KeyDown, CGEventType::KeyUp],
            |_proxy, event_type, event| {
                handle_event(event_type, event);
                None // ListenOnly: return None (event is not modified)
            },
        );

        match tap {
            Ok(tap) => {
                HOOK_ACTIVE.store(true, Ordering::Relaxed);
                tracing::info!("CGEventTap installed (macOS keyboard hook active)");

                let loop_source = tap
                    .mach_port
                    .create_runloop_source(0)
                    .expect("Failed to create CFRunLoop source from CGEventTap");

                let run_loop = CFRunLoop::get_current();
                run_loop.add_source(&loop_source, unsafe { kCFRunLoopDefaultMode });
                tap.enable();
                CFRunLoop::run_current();

                // CFRunLoopRun returned — tap was disabled or invalidated.
                HOOK_ACTIVE.store(false, Ordering::Relaxed);
                tracing::warn!("CGEventTap CFRunLoop exited unexpectedly.");
            }
            Err(_) => {
                tracing::error!(
                    "CGEventTap creation failed even though preflight returned true. \
                     This may indicate a sandboxing or TCC database issue."
                );
            }
        }
    });
}
