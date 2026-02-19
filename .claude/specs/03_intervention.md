Layer 3: Intervention UI Specification
Objective
Provide visual feedback (Nudge, Fade, Wall) without stealing focus or blocking interaction.

Window Management Strategy (Multi-Window)
We need two distinct windows managed by Tauri:

overlay_window (Passive):

Role: Ambient Fade, Nudge borders.

Properties: Fullscreen, Transparent, AlwaysOnTop, Click-through (Ignore Mouse Events).

Implementation:

Use window.set_ignore_cursor_events(true) API in Tauri v2.

Draw semi-transparent fog/particles using CSS/Canvas.

wall_window (Active - Block):

Role: The Wall (Lock screen).

Properties: Fullscreen, Transparent background (blur), AlwaysOnTop, Captures Mouse/Keyboard.

Implementation:

When STUCK level is Critical (Lv4), show this window and focus it.

Block Alt+Tab / Win keys via Low-level Hook (only when Wall is active).

Interaction Logic
The overlay_window must allow the user to type in Word/VSCode underneath it.

Communication between Rust (Inference) and Frontend (Overlay) via tauri::emit

Let's think step by step.