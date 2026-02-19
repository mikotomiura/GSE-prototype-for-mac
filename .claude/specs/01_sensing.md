Sensing Layer Specification
Objective
Capture user keystrokes globally and extract rhythm features without blocking the UI.

Requirements
1.Global Hook:

Implement WH_KEYBOARD_LL using SetWindowsHookEx.

Must run in a dedicated background thread (not the Tauri main thread).

Communicate with the main thread via mpsc::channel.

2.IME State Detection:

Detect if IME is ON/OFF using ImmGetOpenStatus.

Detect if "Conversion Mode" is active (Candidate window visible). Since TSF global hooks are blocked, use a heuristic:

If VK_CONVERT or VK_SPACE (in IME mode) is pressed, set flag is_converting = true.

If VK_RETURN is pressed, is_converting = false.

3.Feature Extraction (Real-time):

Maintain a sliding window (30 seconds) of key events.

Calculate F1 (Flight Time Median) and F6 (Pause-after-Delete).

F6 Logic: If VK_BACK is released, start a timer. If next key > 2000ms, increment Stuck count.

Rust Crate Dependencies
windows (Win32::UI::WindowsAndMessaging, Win32::UI::Input::Ime)

lazy_static or once_cell for hook handle storage.

Let's think step by step.