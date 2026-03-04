# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

GSE (Generative Struggle Engine) — a Tauri 2.0 + Rust + React/TypeScript prototype that estimates cognitive state (Flow / Incubation / Stuck) from keystroke dynamics using a hand-tuned Hidden Markov Model. No cloud dependency. Currently macOS-only (Windows port planned as primary target).

## Build Commands

```bash
# Development (Vite dev server + Tauri app)
npm install
npm run tauri dev

# Release build (outputs to src-tauri/target/release/bundle/macos/)
npm run tauri build

# Rust-only compile check (no UI)
cd src-tauri && cargo build --target aarch64-apple-darwin
```

**Prerequisites:** Rust 1.77+, Node.js 20+, Tauri CLI v2 (`cargo install tauri-cli --version "^2"`), Xcode CLI tools. macOS Input Monitoring permission required at runtime.

**Note:** `.cargo/config.toml` sets `jobs = 1` (prevent OOM on 8GB machines) and `RUST_MIN_STACK = 64MB` (deep macro expansion in Tauri 2.x transitive deps).

There are no tests in this codebase currently.

## Architecture

### Thread Model (6 threads spawned from `lib.rs`)

```
Main Thread (Tauri event loop + GCD main queue for TIS)
├─ Hook Thread         ← CGEventTap + CFRunLoop (dedicated thread)
│       │ crossbeam::channel (bounded 64, non-blocking send)
│       │ bounded(1) wake channel → IME Polling Thread
├─ Analysis Thread     ← recv_timeout(1s) → 1Hz timer gate; HMM update ≤ 1×/sec
├─ IME Monitor Thread  ← reads IME_ACTIVE every 100ms (set by Hook Thread state machine)
├─ IME Polling Thread  ← recv_timeout(100ms) wake; TIS dispatch_sync_f to main queue
├─ Logger Thread       ← bounded channel(512) → NDJSON file (BufWriter)
└─ Sensor Thread       ← stub (accelerometer not implemented on macOS)
```

All inter-thread communication uses `crossbeam` bounded channels. Shared state uses `Arc<AtomicBool>`, `Arc<AtomicU32>`, or `Arc<Mutex<T>>`. Mutex poisoning is handled throughout with `unwrap_or_else(|p| p.into_inner())`.

### Rust Backend (`src-tauri/src/`)

- **`lib.rs`** — Tauri setup, thread orchestration, 8 IPC commands (`get_cognitive_state`, `get_keyboard_idle_ms`, `get_hook_status`, `start_wall_server`, `stop_wall_server`, `get_session_file`, `start_session`, `quit_app`)
- **`analysis/engine.rs`** — HMM engine: 3 states × 26 observation bins (25 natural + 1 backspace penalty), dual semantic latent axes (Friction, Engagement), EWMA smoothing (α=0.3), hysteresis display layer (α=0.40/0.60)
- **`analysis/features.rs`** — F1-F6 feature extraction over 30-second sliding window, `phi(x, beta)` normalization (κ=2.0), `make_silence_observation()` for idle periods
- **`input/hook.rs`** — Shared atomics (`IME_OPEN`, `IME_ACTIVE`, `IME_COMPOSING`, `HOOK_ACTIVE`, `EVENT_SENDER`). Uses `#[path = "hook_macos.rs"]` for platform dispatch
- **`input/hook_macos.rs`** — CGEventTap implementation, macOS VK→Windows VK mapping (~60 codes), Input Monitoring permission flow, **IME composition state machine** (tracks COMPOSING→CANDIDATING transitions via keystroke patterns)
- **`input/ime.rs`** — `ImeMonitor` reads `IME_ACTIVE` from hook state machine + `spawn_ime_open_polling_thread()` with JIS key vs ANSI keyboard detection paths
- **`input/ime_macos.rs`** — TIS Carbon FFI via `dispatch_sync_f` to main GCD queue
- **`logger.rs`** — NDJSON session logger to `~/Documents/GSE-sessions/`, auto-flush on 5s idle
- **`wall_server.rs`** — Embedded HTTP server (tiny_http) + QR code for smartphone-based Zen Timer (2-min countdown → unlock button)
- **`sensors.rs`** / **`sensors_macos.rs`** — Stub dispatcher

### Frontend (`src/`)

- **`App.tsx`** — Polls `get_cognitive_state` every 500ms, intervention state machine (Lv1 Nudge at stuck>0.60, Lv2 Wall at stuck>0.70 for 30s continuous)
- **`components/Dashboard.tsx`** — State probability bars, hook status banner, quit button
- **`components/Overlay.tsx`** — Transparent always-on-top window: red vignette nudge (mix-blend-mode: hard-light) + full-screen Wall with QR code (smartphone unlock only, no PC-side auto-unlock)

Two Tauri windows: `main` (Dashboard) and `overlay` (transparent, always-on-top, click-through when nudge / blocking when wall).

### Key HMM Details

- Engine is 1Hz-calibrated: `engine.update()` must be called at most once per second
- All VK codes are normalized to Windows equivalents internally (macOS CGKeyCode → Windows VK in `macos_vk_to_vk()`)
- Backspace streak ≥ 8 triggers penalty bin (obs=25) with separate hysteresis alpha (0.60)
- Dual baseline: `beta_coding` (IME off) vs `beta_writing` (IME on) with different reference values
- Emission table entries have baked-in minimum of 0.01 (no runtime additive floor)
- `force_flow_state()` resets to Flow when IME candidate window is active

## Critical Constraints

- **Tauri 2.0 Capabilities** — Use capabilities system (`src-tauri/capabilities/`), NOT v1 allowlist
- **`windows` crate (v0.58+)** — NEVER use `winapi` crate
- **Logging** — Use `tracing` crate, not `println!` or `log`
- **Platform dispatch** — `#[path = "file.rs"]` required when declaring submodules from a `.rs` file (not directory `mod.rs`)
- **CGEventTap** — `CGEventType` in core-graphics 0.23 does NOT impl `PartialEq`, use `matches!()`; placement is `TailAppendEventTap` for listen-only taps
- **TIS FFI** — Must use `dispatch_sync_f` to main GCD queue; direct calls crash with `dispatch_assert_queue(main_queue)`
- **tauri.conf.json** — `bundle.macOS.infoPlist` expects a file path string, not inline JSON
- **macOS private API** — `macOSPrivateApi: true` required in tauri.conf.json for transparent overlay window

## Git Conventions

Conventional commits: `feat(scope):`, `fix(scope):`, `refactor:`, `chore:`, `docs:`
Scopes: `wall`, `macos/ime`, `overlay`, `features`, `build`, `ui`, `sensors`

## macOS Limitations (vs planned Windows)

- `IME_ACTIVE` (candidate window detection) uses keystroke state machine in CGEventTap callback — detects Space during composition as candidate trigger, Enter/Escape as reset. No extra permissions needed. Heuristic: may briefly misdetect if user presses Space for non-conversion purposes while IME_OPEN
- Accelerometer stubbed → Wall requires QR/smartphone Zen Timer unlock (no PC-side auto-unlock)
- First run requires Input Monitoring permission grant + app restart
- No Windows source files exist yet (`windows_impl.rs`, `windows_ime.rs`, `sensors_windows.rs` are planned)
