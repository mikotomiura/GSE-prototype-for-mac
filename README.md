# GSE — Generative Struggle Engine for macOS

> **Real-time cognitive state estimation from keystroke dynamics on macOS**
> Classifies the writer's mental state into **Flow / Incubation / Stuck** using a hand-tuned Hidden Markov Model, without any cloud dependency or user-facing interruption.

[🇯🇵 日本語版 README はこちら](README.ja.md)

---

## Table of Contents

1. [Motivation](#motivation)
2. [Cognitive State Model](#cognitive-state-model)
3. [Architecture Overview](#architecture-overview)
4. [Folder Structure](#folder-structure)
5. [Feature Extraction (F1–F6)](#feature-extraction-f1f6)
6. [HMM Engine](#hmm-engine)
7. [Hysteresis & Stability Fixes (v2.1)](#hysteresis--stability-fixes-v21)
8. [IME Detection and Context-Aware Baseline (v2.2)](#ime-detection-and-context-aware-baseline-v22)
9. [IME Mode Detection on macOS: TIS + JIS Key Architecture](#ime-mode-detection-on-macos-tis--jis-key-architecture)
10. [1Hz Timer-Driven Inference & Synthetic Friction (v2.4)](#1hz-timer-driven-inference--synthetic-friction-v24)
11. [Intervention UI: Nudge & Wall (v2.5)](#intervention-ui-nudge--wall-v25)
12. [Logging & Analysis](#logging--analysis)
13. [Build Instructions](#build-instructions)
14. [Academic References](#academic-references)

---

## Motivation

Writers, programmers, and knowledge workers alternate between states of **flow** (effortless, high-output), **incubation** (deliberate pause, sub-conscious processing), and **stuck** (cognitive block, unproductive looping). Real-time awareness of these states could enable adaptive tools—ambient music, nudges, or UI dimming—to scaffold metacognition without disrupting the task itself.

Existing approaches require wearables, cameras, or explicit self-report. This prototype uses only **keystroke timing** (already available from the OS), making it deployable on any macOS device without additional hardware.

---

## Cognitive State Model

The three states are grounded in established cognitive science literature:

| State | Definition | Behavioral Signature |
|---|---|---|
| **Flow** | Effortless, intrinsically motivated task engagement (Csikszentmihalyi, 1990) | Short inter-key intervals, low correction rate, long continuous bursts |
| **Incubation** | Deliberate pause enabling sub-conscious problem restructuring (Sio & Ormerod, 2009) | **High $P(\text{Burst} \mid \text{Pause})$**: Extended silence (≥2 s) followed by rapid output burst |
| **Stuck** | Perseverative failure to escape an impasse (Ohlsson, 1992) | **High $P(\text{Pause} \mid \text{Delete})$**: Perseverative delete-pause loops with near-zero character gain |

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         macOS (Apple Silicon / Intel)               │
│                                                                      │
│  ┌─────────────┐   CGEventTap       ┌──────────────────────────┐   │
│  │  Any App    │ ─────────────────── │   Hook Thread (Rust)     │   │
│  │ (foreground)│  (Input Monitoring) │  CGEventTapCreate        │   │
│  └─────────────┘                     │  CFRunLoop (dedicated)   │   │
│                                      └──────────┬───────────────┘   │
│                                                 │ crossbeam channel  │
│                                      ┌──────────▼───────────────┐   │
│                                      │  Analysis Thread (Rust)  │   │
│                                      │  ── 1 Hz Timer Gate ──   │   │
│                                      │  FeatureExtractor        │   │
│                                      │    F1 flight-time median │   │
│                                      │    F2 flight-time var.   │   │
│                                      │    F3 correction rate    │   │
│                                      │    F4 burst length       │   │
│                                      │    F5 pause count        │   │
│                                      │    F6 pause-after-del.   │   │
│                                      │                          │   │
│                                      │  CognitiveStateEngine    │   │
│                                      │    Latent Axes (X, Y)    │   │
│                                      │    EWMA smoothing        │   │
│                                      │    HMM Forward Step      │   │
│                                      │    Hysteresis EMA layer  │   │
│                                      └──────────┬───────────────┘   │
│                                                 │ Tauri IPC          │
│                                      ┌──────────▼───────────────┐   │
│                                      │   React/TS Frontend      │   │
│                                      │   Dashboard + Overlay    │   │
│                                      │   Lv1 Nudge (red mist)   │   │
│                                      │   Lv2 Wall  (full block) │   │
│                                      └──────────────────────────┘   │
│                                                 │                    │
│                                      ┌──────────▼───────────────┐   │
│                                      │   SessionLogger (Rust)   │   │
│                                      │   NDJSON → Documents/    │   │
│                                      │   GSE-sessions/          │   │
│                                      └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

### Thread Model

```
Main Thread (Tauri event loop + GCD main queue for TIS)
    │
    ├─ Hook Thread         ← CGEventTap callback + CFRunLoop (dedicated thread)
    │       │ crossbeam::channel (bounded 64, non-blocking send)
    │       │ bounded(1) wake channel → IME Polling Thread
    ├─ Analysis Thread     ← recv_timeout(1 s) → 1 Hz timer gate; HMM update ≤ 1×/sec
    │       │ Arc<Mutex<CognitiveStateEngine>> (Tauri managed state)
    │       │ Synthetic Friction: silence ≥ 20 s → ramps F6/F3 toward Stuck
    ├─ IME Monitor Thread  ← polls is_candidate_window_open() every 100 ms (stub: always false)
    ├─ IME Polling Thread  ← recv_timeout(100 ms) wake; TIS dispatch_sync_f to main queue
    │
    ├─ Logger Thread       ← bounded channel(512) → NDJSON file (BufWriter)
    └─ Sensor Thread       ← stub (accelerometer not implemented on macOS)
```

---

## Folder Structure

```
GSE-prototype/
├── analysis/
│   ├── behavioral_gt.py         # Post-session behavioral GT labeling
│   └── hmm_sensitivity.py       # Parameter sensitivity analysis
│
├── src/                         # React / TypeScript frontend
│   ├── components/
│   │   ├── Dashboard.tsx        # State probability bars + session info
│   │   └── Overlay.tsx          # Nudge (red mist) + Wall (full-screen block)
│   ├── App.tsx                  # Intervention state machine (Lv1 → Lv2 escalation)
│   └── main.tsx
│
├── src-tauri/                   # Rust / Tauri 2.0 backend
│   ├── capabilities/
│   │   └── default.json         # Tauri 2.0 capability declarations
│   ├── src/
│   │   ├── analysis/
│   │   │   ├── engine.rs        # HMM + hysteresis layer (display_probs EMA)
│   │   │   ├── features.rs      # F1–F6 extraction + silence synthesis
│   │   │   └── mod.rs
│   │   ├── input/
│   │   │   ├── hook.rs          # Shared statics (IME_OPEN, EVENT_SENDER, etc.)
│   │   │   ├── hook_macos.rs    # CGEventTap implementation (macOS keyboard hook)
│   │   │   ├── ime.rs           # ImeMonitor stub + TIS polling dispatcher
│   │   │   ├── ime_macos.rs     # TIS Carbon FFI (dispatch_sync_f to main queue)
│   │   │   └── mod.rs
│   │   ├── lib.rs               # Tauri setup, thread orchestration, IPC commands
│   │   ├── logger.rs            # Async NDJSON session logger
│   │   ├── main.rs
│   │   ├── sensors.rs           # Sensor dispatcher (macOS stub)
│   │   └── sensors_macos.rs     # Accelerometer stub (not implemented)
│   ├── entitlements.mac.plist   # No app sandbox (research prototype)
│   ├── Info.mac.plist           # NSInputMonitoringUsageDescription
│   ├── Cargo.toml
│   └── tauri.conf.json
│
├── index.html
├── package.json
├── tsconfig.json
└── vite.config.ts
```

---

## Feature Extraction (F1–F6)

All features are computed over a **30-second sliding window** of raw keystroke events, updated on every key press. During silence (no input), a synthetic observation is generated every 1 second via `make_silence_observation()` to keep the HMM running.

| Feature | Symbol | Definition | Cognitive Signal |
|---|---|---|---|
| Flight Time Median | **F1** | Median of key-release → key-press intervals (ms), last 5 samples | Typing speed — lower = Flow |
| Flight Time Variance | **F2** | Variance of flight times within 30-second window | Rhythmic consistency |
| Correction Rate | **F3** | (Backspace + Delete presses) / total presses | Error frequency — higher = Stuck |
| Burst Length | **F4** | Mean length of consecutive keystroke runs (inter-key gap < 200 ms) | Output fluency — higher = Flow |
| Pause Count | **F5** | Count of inter-press gaps ≥ 2 000 ms within window | Deliberation frequency |
| Pause-after-Delete Rate | **F6** | Fraction of Backspace/Delete presses followed by a ≥ 2 s gap | Post-error freeze — higher = Stuck |

### Normalization: φ(x, β)

Each raw feature value is mapped to [0, 1] using a baseline-relative linear normalization:

```
φ(x, β) = clamp( (x − β) / (κ · β), 0.0, 1.0 )     κ = 2.0
```

where β is a fixed reference value representing the expected population median.
Values below β return 0.0; values at 3β return 1.0.

---

## HMM Engine

### Semantic Latent Axes: Cognitive Friction × Productive Silence

The six normalized features are projected onto two interpretable semantic latent axes before discretization:

```text
X (Cognitive Friction)   = 0.30·φ(F3) + 0.25·φ(F6) + 0.25·φ(F1) + 0.20·φ(F5)
Y (Productive Silence)   = 0.40·φ(F4) + 0.35·(1 − φ(F1)) + 0.25·(1 − φ(F5))
```

Both axes are smoothed with an Exponential Weighted Moving Average (α = 0.30):
```
ewma_t = 0.30 · raw_t + 0.70 · ewma_{t−1}
```

### Observation Bins

(X, Y) ∈ [0,1]² is discretized into a 5×5 grid (25 bins) plus one penalty bin (obs = 25, triggered by ≥ 5 consecutive Backspace presses):

```
Cognitive Friction X →  0(low)   1      2      3      4(high)
Productive Silence Y ↓
4 (high)     [Flow]  [Flow]  [   ]  [   ]  [    ]
3            [Flow]  [Flow]  [   ]  [   ]  [    ]
2            [    ]  [    ]  [ ? ]  [Stk]  [Stk ]
1            [Inc ]  [Inc ]  [Inc]  [Stk]  [Stk ]
0 (low)      [Inc ]  [Inc ]  [ ? ]  [Stk]  [Stk ]
```

### HMM Forward Step

At each update, the belief vector **π** = [p_Flow, p_Inc, p_Stuck] is propagated by a single Forward Algorithm step:

```
π'_j = ( Σ_i  π_i · A[i,j] ) · ( B[j, obs] + ε )    for j ∈ {0, 1, 2}
π'   ← π' / Σ_j π'_j                                  (normalize to sum = 1)
```

- **A** = 3×3 transition matrix (rows = from, cols = to)
- **B** = 3×26 emission matrix (state × observation bin)
- **ε** = 0.04 (emission floor)

### Transition Matrix A

| From \ To | Flow | Incubation | Stuck |
|---|---|---|---|
| **Flow** | 0.80 | 0.13 | 0.07 |
| **Incubation** | 0.12 | 0.80 | 0.08 |
| **Stuck** | 0.06 | 0.18 | 0.76 |

---

## Hysteresis & Stability Fixes (v2.1)

### Fix ①: Cold-Start Hysteresis

**Problem:** A 30-second window advancing past heavy backspacing caused a raw HMM belief spike from `p_stuck = 0.994 → p_flow = 0.48` in one step (< 1 ms).

**Fix:** A secondary probability vector `display_probs` is maintained alongside the raw HMM belief, tracked via a slow EMA:

```
display_t = α · raw_t + (1 − α) · display_{t−1}

α = 0.25  (normal updates → time-constant τ ≈ 4 s)
α = 0.50  (backspace-penalty bin → rapid Stuck onset)
```

### Fix ②: Probability Discrete Clustering

**Fix:** Emission floor raised from 0.01 to **0.04**, adding equal additive smoothing to all state likelihoods. Maximum attainable probability per state saturates near 0.88–0.90.

### Fix ③: Inc → Stuck Silence Transition

**Fix:** `make_silence_observation()` now generates synthetic friction values that increase linearly with silence duration:

```rust
// F6 onset at 20 s → reaches 0.50 at 80 s
F6_synthetic = clamp((silence_secs − 20) / 60,  0.0, 0.50)

// F3 onset at 30 s → reaches 0.40 at 130 s
F3_synthetic = clamp((silence_secs − 30) / 100, 0.0, 0.40)
```

| Silence | F3_syn | F6_syn | X (Friction) | x_bin | Region |
|---|---|---|---|---|---|
| 20 s | 0.00 | 0.00 | ≈ 0.20 | 1 | Incubation |
| 30 s | 0.00 | 0.17 | ≈ 0.30 | 1 | Incubation |
| 40 s | 0.10 | 0.33 | ≈ 0.52 | 2 | Boundary |
| 50 s | 0.20 | 0.50 | ≈ 0.75 | 3 | **Stuck** |

---

## IME Detection and Context-Aware Baseline (v2.2)

Japanese IME input introduces two distinct challenges for keystroke analysis:

1. **Candidate selection phase**: navigation keystrokes should not pollute feature vectors
2. **Input mode context** (あ mode vs A mode): flight-time norms differ significantly

### Candidate Window Suppression

On macOS, `ImeMonitor::is_candidate_window_open()` returns `false` (stub). The HMM continues running during candidate selection — this is a known limitation.

### IME Mode Detection (macOS)

On macOS, IME mode is detected through two complementary mechanisms:

| Mechanism | Description | Latency |
|---|---|---|
| **JIS key detection** | `kVK_JIS_Eisu` (0x66) / `kVK_JIS_Kana` (0x68) detected in CGEventTap callback | < 5 ms |
| **TIS polling** | `TISCopyCurrentKeyboardInputSource()` polled every 100 ms (wake-on-keystroke) | ≤ 100 ms |

**Thread safety:** TIS Carbon APIs must run on the GCD main queue. `is_japanese_ime_open()` marshals the call via `dispatch_sync_f(&_dispatch_main_q, ...)` to avoid the `dispatch_assert_queue(main_queue)` crash.

The input source ID is checked for `"inputmethod.Japanese"` (covers Apple IME, Google Japanese Input, etc.).

**ANSI/US keyboards:** JIS HID codes (`kVK_JIS_Eisu`, `kVK_JIS_Kana`) will not appear. TIS polling (100 ms) is the sole detection path — both keyboard types work correctly.

### Context-Specific Baseline β (Dual Baseline)

| Context | `IME_OPEN` | β_F1 (ms) | β_F3 | β_F4 | β_F5 | β_F6 |
| --- | --- | --- | --- | --- | --- | --- |
| **β_coding** (A mode) | `false` | 150 | 0.06 | 5.0 | 2.0 | 0.08 |
| **β_writing** (あ/カ mode) | `true` | 220 | 0.08 | 2.0 | 4.0 | 0.12 |

---

## IME Mode Detection on macOS: TIS + JIS Key Architecture

### Anti-Flapping Design

`LogEntry::ImeState` is emitted exclusively from the **IME Polling Thread**, never from the hook callback or analysis thread:

```
CGEventTap callback (Hook Thread)
    │  atomic store: IME_OPEN = new_state  (JIS keys only)
    │  atomic store: IME_STATE_DIRTY = true
    └─ try_send(()) on bounded(1) wake channel

IME Polling Thread
    │  recv_timeout(100ms)   ← wakes within 1ms of keypress
    │  sleep(5ms)            ← lets OS settle IME state
    │  dispatch_sync_f → main queue: TISCopyCurrentKeyboardInputSource()
    │  compare with last_state (Option<bool>)
    └─ emit LogEntry::ImeState { timestamp: SystemTime::now(), on }
```

### Key Atomics (`hook.rs`)

| Atomic | Writer | Reader | Purpose |
|---|---|---|---|
| `IME_OPEN` | hook (JIS keys), polling thread | analysis thread | Current IME mode |
| `IME_STATE_DIRTY` | hook (JIS keys) | polling thread (drain) | Housekeeping |
| `IME_ACTIVE` | — | ImeMonitor (stub) | Always false on macOS |
| `HOOK_ACTIVE` | `hook_macos::start()` | `get_hook_status` command | Permission banner trigger |

### `Option<bool>` Initial State

The polling thread tracks `last_state: Option<bool> = None`, ensuring the **first detection always emits** regardless of whether the initial state is `false` or `true`.

---

## 1Hz Timer-Driven Inference & Synthetic Friction (v2.4)

The analysis thread enforces a **1 Hz timer gate** via `recv_timeout(1000 ms)`:

- **Keystroke arrives within 1 s:** Features accumulated; `engine.update()` called only if ≥ 1 s elapsed since last HMM step.
- **Timeout:** `make_silence_observation()` generates a synthetic silence observation.

This ensures exactly **one HMM forward step per second**, making the EMA time constant τ = 1/α ≈ **4 seconds** precise regardless of typing speed.

---

## Intervention UI: Nudge & Wall (v2.5)

| Level | Name | Trigger | Visual Effect | User Interaction |
| --- | --- | --- | --- | --- |
| **Lv1** | Nudge | p_stuck > 0.60 | Red vignette (mist) around screen edges | Click-through (transparent to input) |
| **Lv2** | Wall | p_stuck > 0.70 for 30 s | Full-screen blocking overlay with message | Blocks all input until unlocked |

The Wall is unlocked by the **accelerometer** event (`"sensor-accelerometer"` / `"move"` payload). On macOS, the accelerometer is not implemented — the Wall must be dismissed manually via `setIgnoreCursorEvents(false)` toggle (research prototype limitation).

---

## Logging & Analysis

Every session produces a timestamped NDJSON file:

```
~/Documents/GSE-sessions/gse_YYYYMMDD_HHMMSS.ndjson
```

Record types:

```jsonc
// Session metadata
{"type":"meta","session_start":1740000000000}

// Raw keystroke event
{"type":"key","t":1740000001234,"vk":65,"press":true}

// Feature snapshot + HMM state probabilities
{"type":"feat","t":1740000001235,
 "f1":145.20,"f2":312.00,"f3":0.0800,"f4":6.50,"f5":1.0,"f6":0.0000,
 "p_flow":0.7123,"p_inc":0.2100,"p_stuck":0.0777}

// IME mode switch
{"type":"ime_state","t":1740000001234,"on":true}

{"type":"meta","session_end":1740000060000}
```

### Post-session Ground-Truth Labeling

```bash
python analysis/behavioral_gt.py ~/Documents/GSE-sessions/gse_YYYYMMDD_HHMMSS.ndjson
```

| Label | Behavioral Rule |
|---|---|
| **FLOW** | median(FT) < 200 ms AND correction_rate < 0.15 AND not STUCK/INC |
| **INCUBATION** | Pause(≥ 2 s) → Burst(≥ 5 chars at FT < 200 ms) → diff_chars ≥ 3 within 30 s |
| **STUCK** | ≥ 3× (Burst(≤ 3 chars) → Delete(≥ 1) → Pause(≥ 2 s)) in 60 s AND diff_chars ≤ 0 |
| **UNKNOWN** | No condition met, or multiple labels conflict |

---

## Build Instructions

### Prerequisites

| Tool | Version |
|---|---|
| Rust | 1.77+ (`rustup update stable`) |
| Node.js | 20+ |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |
| Xcode Command Line Tools | `xcode-select --install` |

### macOS Permission: Input Monitoring

GSE uses `CGEventTap` to capture keystroke timing. On first launch:

1. macOS will prompt: *"GSE would like to monitor input from your keyboard"*
2. Go to **System Settings → Privacy & Security → Input Monitoring**
3. Enable the toggle next to **GSE**
4. Restart the app

Without this permission, `HOOK_ACTIVE` is `false` and a yellow banner is shown in the Dashboard.

### Development

```bash
cd GSE-prototype
npm install
npm run tauri dev
```

### Release Build

```bash
npm run tauri build
# App bundle: src-tauri/target/release/bundle/macos/
```

### Verify Hook is Active

```bash
# Compile only (no UI)
~/.cargo/bin/cargo build --target aarch64-apple-darwin
```

---

## Known macOS Limitations

| Feature | Status | Impact |
|---|---|---|
| IME composition detection (`IME_ACTIVE`) | `false` (stub) | HMM runs during candidate window |
| Accelerometer unlock | Not implemented | Wall requires manual click-through toggle |
| JIS IME keys (ANSI keyboard) | Detected via TIS polling only (100ms) | Negligible latency difference |
| First-run Input Monitoring permission | Requires restart after granting | One-time setup |

---

## Academic References

1. **Csikszentmihalyi, M.** (1990). *Flow: The Psychology of Optimal Experience*. Harper & Row.
2. **Csikszentmihalyi, M.** (1996). *Creativity: Flow and the Psychology of Discovery and Invention*. HarperCollins.
3. **Sio, U. N., & Ormerod, T. C.** (2009). Does incubation enhance problem solving? *Psychological Bulletin, 135*(1), 94–120.
4. **Ohlsson, S.** (1992). Information-processing explanations of insight and related phenomena. In *Advances in the Psychology of Thinking* (pp. 1–44). Harvester Wheatsheaf.
5. **Rabiner, L. R.** (1989). A tutorial on hidden Markov models. *Proceedings of the IEEE, 77*(2), 257–286.
6. **Dhakal, V., Feit, A. M., Kristensson, P. O., & Oulasvirta, A.** (2018). Observations on typing from 136 million keystrokes. *CHI 2018*.
7. **Salthouse, T. A.** (1986). Perceptual, cognitive, and motoric aspects of transcription typing. *Psychological Bulletin, 99*(3), 303–319.
8. **Apple Inc.** (2023). *Text Input Sources Reference (Carbon)*. Apple Developer Documentation. — `TISCopyCurrentKeyboardInputSource`, `kTISPropertyInputSourceID`; used for cross-process IME mode detection.

---

## License

Research prototype. All rights reserved.

---

*Last updated: 2026-02-28*
