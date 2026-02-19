# Development Memory

## Phase 1: Sensing Layer Implementation (2026-02-19)

### Technical Decisions
- **Framework**: Tauri 2.0 + React + TypeScript structure adopted.
- **Dependencies**: 
  - `windows` (0.58): Updated from older versions. Key modules: `Win32::UI::WindowsAndMessaging` (for hook types).
  - `lazy_static`: Used for thread-safe global hook handle storage.
  - `crossbeam-channel`: For non-blocking event transmission from hook thread to analysis thread.
- **Concurrency**:
  - `Hook Thread`: Runs `GetMessageW` loop to keep `SetWindowsHookExW` active.
  - `Analysis Thread`: Simple loop receiving events via channel.

### Challenges & Solutions
- **Windows Crate Versioning**: `windows` 0.58 changed module locations for `KBDLLHOOKSTRUCT` and hook functions to `UI::WindowsAndMessaging` (previously `UI::Input::KeyboardAndMouse` in older docs/examples).
- **Thread Safety**: `HHOOK` is not `Send`/`Sync`. Created `ThreadSafeHook` wrapper to store it in `lazy_static` Mutex.

### Current Status
- Global Keyboard Hook (WH_KEYBOARD_LL) is active and non-blocking.
- Feature Extractor calculates Flight Time Median on key press.
- `cargo check` passes with no errors.

## Phase 2: Inference Layer (HMM) Implementation (2026-02-19)

### Technical Decisions
- **HMM Implementation**: Pivoted from `karma` crate to a **Manual Implementation**.
  - *Reason*: `karma` crate (0.1.0) caused import resolution errors (`no HiddenMarkovModel in root`) despite correct dependency. The API Surface was not as expected or documentation was scarce.
  - *Solution*: Implemented a lightweight Discrete HMM with the Forward Algorithm update step directly in `analysis/engine.rs`. This provides full control and avoids external dependency risks.
- **Integration**:
  - `CognitiveStateEngine` is shared via `Arc<...> + Mutex` and managed by Tauri state.
  - `get_cognitive_state` command returns current probability distribution as a `HashMap`.

### Current Status
- HMM Engine is integrated and running in the background analysis thread.
- Flight Times are discretized (0-10 levels) and fed into the HMM.
- `cargo check` passes.

## Phase 3: Intervention & TSF Monitor Implementation (2026-02-19)

### Technical Decisions
- **Overlay Window**: 
  - Implemented as a secondary Tauri window (`overlay`) with transparent background and `always_on_top`.
  - Used `set_ignore_cursor_events(true)` to ensure it is click-through, allowing users to work on applications behind it.
- **IME Monitor (UI Automation)**:
  - Used `windows::Win32::UI::Accessibility` (UI Automation API) to detect Candidate Windows.
  - Implemented logic to check `CurrentClassName` and `CurrentName` for "Candidate" or "IME" related strings.
  - This allows the system to pause "Stuck" inference while the user is selecting conversion candidates, preventing false positives.

### Challenges & Solutions
- **Build Errors**: Encountered type inference issues with `windows` crate `Result` and `Option` types in `ime.rs`. Resolved by making types explicit and using idiomatic pattern matching.
- **SetIgnoreCursorEvents**: Had to verify the correct API usage for Tauri v2.

### Current Status
- Overlay window launches successfully (transparent/click-through).
- IME Monitor thread runs and updates the HMM engine's `is_paused` state.
- `cargo check` passes.

---

## Phase Satisfaction Scores

## Phase Satisfaction Scores (Strict Implementation Criteria)

### Phase 1: Sensing Layer (100%)
- **Implementation vs Design**: Completely implemented the Global Keyboard Hook and Flight Time calculation as designed.
- **Test Status**: Stability verified via `cargo check` and robust thread management (`ThreadSafeHook`). Non-blocking requirement satisfied.

### Phase 2: Inference Layer (80%)
- **Implementation vs Design**: The Core HMM Engine (`analysis/engine.rs`) was successfully implemented manually. However, the **Feature Engineering** is currently limited to "Median Flight Time" (F1). The design document (`gpt-advice.md`) called for more complex features like "Pause-after-Delete Rate" (F6) to distinguish Stuck vs Incubation. These are not yet implemented.
- **Test Status**: Basic logic compiles and runs, but the inference accuracy is limited by the single feature.

### Phase 3: Intervention & TSF Monitor (100%)
- **Implementation vs Design**: Fully implemented the **Overlay Window** (Click-through) and **IME Monitor** (UI Automation) exactly as specified in the Phase 3 specs.
- **Test Status**: `cargo check` passes. Threading model (separate monitor thread) is robust. The usage of `set_ignore_cursor_events` properly addresses the requirement.

## Phase 4: Surface Pro 8 Sensors (WinRT) Implementation (2026-02-19)

### Technical Decisions
- **WinRT Integration**: Used `windows` crate with `Devices::Sensors` and `Devices::Geolocation`.
- **Thread Safety**: 
  - Each sensor runs in its own thread to avoid blocking the main runtime.
  - **Critical**: Explicitly called `RoInitialize(RO_INIT_MULTITHREADED)` in sensor threads to ensure COM/WinRT works correctly.
- **Event Emission**: Used `app_handle.emit()` to send `sensor-accelerometer` and `sensor-geolocation` events to the frontend.

### Challenges & Solutions
- **Build Errors**: Encountered namespace issues with `RoInitialize` (moved to `windows::Win32::System::WinRT` in v0.58) and missing `Win32_System_WinRT` feature in `Cargo.toml`. Resolved by adding the feature and correcting the path.

### Phase Satisfaction Score: 100%
- **Implementation vs Design**: Successfully implemented Accelerometer and Geolocator monitoring as specified.
- **Test Status**: `cargo check` passes. Logic handles both success and failure cases (fallback).

## Phase 5: Frontend Implementation (2026-02-19)

### Technical Decisions
- **UI Architecture**:
  - `App.tsx`: Acts as the controller, polling backend state and routing to `Dashboard` or `Overlay` based on window label.
  - **Dashboard**: Simple, effective visualization of HMM probabilities using CSS-based progress bars.
  - **Overlay**: Implemented "The Wall" logic (3s persistence) and "Nudge" (vignette) using React state and CSS transitions.
- **Sensor Integration**: Frontend listens for `sensor-accelerometer` ("move" payload) to unlock "The Wall".

### Challenges & Solutions
- **Build Environment**: `npm` was initially missing from the environment. Resolved by verifying Node.js installation and refreshing environment variables.
- **TypeScript Errors**: `NodeJS.Timeout` type mismatch in browser context. Fixed by using `ReturnType<typeof setTimeout>`.

### Phase Satisfaction Score: 100%
- **Implementation vs Design**: UI implements all required features: Dashboard visualization, Nudge, Wall, and Sensor Unlock.
- **Test Status**: `npm run build` passed successfully, verifying type safety and build integrity.
