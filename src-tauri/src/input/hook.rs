use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crossbeam_channel::Sender;
use lazy_static::lazy_static;
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
    WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::analysis::features::InputEvent;

// Wrapper to make HHOOK Send+Sync for lazy_static
struct ThreadSafeHook(#[allow(dead_code)] HHOOK);
unsafe impl Send for ThreadSafeHook {}
unsafe impl Sync for ThreadSafeHook {}

lazy_static! {
    static ref HOOK_HANDLE: Mutex<Option<ThreadSafeHook>> = Mutex::new(None);
    static ref EVENT_SENDER: Mutex<Option<Sender<InputEvent>>> = Mutex::new(None);
}

pub fn init_hook(sender: Sender<InputEvent>) {
    // Store sender
    {
        let mut s = (*EVENT_SENDER).lock().unwrap();
        *s = Some(sender);
    }

    // Spawn hook thread
    thread::spawn(|| {
        unsafe {
            let hook_id = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(hook_callback),
                HINSTANCE::default(),
                0,
            );

            match hook_id {
                Ok(h) => {
                    {
                        let mut handle = (*HOOK_HANDLE).lock().unwrap();
                        *handle = Some(ThreadSafeHook(h));
                    }
                    eprintln!("Keyboard hook installed");

                    // Message loop is required for the hook to work
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        // In a real app we might translate/dispatch,
                        // but for a pure hook thread, we just need to pump messages.
                    }
                }
                Err(e) => {
                    eprintln!("Failed to install keyboard hook: {:?}", e);
                }
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

            let event = InputEvent {
                vk_code,
                timestamp,
                is_press,
            };

            // Non-blocking send attempt
            if let Ok(guard) = (*EVENT_SENDER).try_lock() {
                if let Some(sender) = guard.as_ref() {
                    let _ = sender.try_send(event);
                }
            }
        }
    }

    CallNextHookEx(None, code, wparam, lparam)
}
