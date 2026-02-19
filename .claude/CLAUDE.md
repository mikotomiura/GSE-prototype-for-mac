GSE (Generative Struggle Engine) Project Guidelines
Project Identity
Goal: Build a cognitive state estimation system (Flow/Incubation/Stuck) based on keystroke dynamics.

Target: Windows 11 (Surface Pro 8), Touch/Type Cover.

Stack: Tauri 2.0, Rust (Backend), React/TypeScript (Frontend), ONNX Runtime.

🛡️ Critical Technical Constraints (Strict)
1.Windows API Safety:

Use windows crate (v0.58+). NEVER use winapi.

All unsafe blocks for Win32 APIs must be isolated in wrapper structs.

NO Global TSF Hooks: Do not attempt to hook ITfThreadMgr globally (blocked by UIPI). Use SetWindowsHookEx for keys and UIAutomation for candidate window detection.

2.Performance & Concurrency:

Non-blocking Input: The keyboard hook (WH_KEYBOARD_LL) MUST run in a dedicated thread. It must send events via crossbeam::channel to the analysis thread. NEVER block the hook callback.

Async: Use tokio for heavy tasks (Inference, I/O).

3.Tauri 2.0 Specifics:

Use tauri-plugin-store for persistence.

Use Capabilities (capabilities/default.json) for permissions. Do not rely on v1 allowlist.

4.🤖 Coding Style
Rust: Prefer anyhow for app-level errors. Use tracing for logging.

Frontend: Use shadcn/ui concepts. Minimal dependencies.

5.Documentation: Update docs/ when architecture changes.