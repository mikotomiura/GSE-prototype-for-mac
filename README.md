# GSE-Next — Generative Struggle Engine (Prototype v2)

> **Real-time cognitive state estimation from keystroke dynamics on Windows 11**
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
9. [IME Mode Detection: Robust Multi-Layer Architecture (v2.3)](#ime-mode-detection-robust-multi-layer-architecture-v23)
10. [1Hz Timer-Driven Inference & Synthetic Friction (v2.4)](#1hz-timer-driven-inference--synthetic-friction-v24)
11. [Intervention UI: Nudge & Wall (v2.5)](#intervention-ui-nudge--wall-v25)
12. [Logging & Analysis](#logging--analysis)
13. [Build Instructions](#build-instructions)
14. [Academic References](#academic-references)

---

## Motivation

Writers, programmers, and knowledge workers alternate between states of **flow** (effortless, high-output), **incubation** (deliberate pause, sub-conscious processing), and **stuck** (cognitive block, unproductive looping). Real-time awareness of these states could enable adaptive tools—ambient music, nudges, or UI dimming—to scaffold metacognition without disrupting the task itself.

Existing approaches require wearables, cameras, or explicit self-report. This prototype uses only **keystroke timing** (already available from the OS), making it deployable on any Windows device without additional hardware.

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
│                         Windows 11 (Surface Pro 8)                  │
│                                                                      │
│  ┌─────────────┐   WH_KEYBOARD_LL    ┌──────────────────────────┐  │
│  │  Any App    │ ─────────────────── │   Hook Thread (Rust)     │  │
│  │ (foreground)│                     │  SetWindowsHookExW       │  │
│  └─────────────┘                     │  WinEvent IME monitor    │  │
│                                      └──────────┬───────────────┘  │
│                                                 │ crossbeam channel │
│                                      ┌──────────▼───────────────┐  │
│                                      │  Analysis Thread (Rust)  │  │
│                                      │  ── 1 Hz Timer Gate ──   │  │
│                                      │  FeatureExtractor        │  │
│                                      │    F1 flight-time median │  │
│                                      │    F2 flight-time var.   │  │
│                                      │    F3 correction rate    │  │
│                                      │    F4 burst length       │  │
│                                      │    F5 pause count        │  │
│                                      │    F6 pause-after-del.   │  │
│                                      │                          │  │
│                                      │  CognitiveStateEngine    │  │
│                                      │    Latent Axes (X, Y)    │  │
│                                      │    EWMA smoothing        │  │
│                                      │    HMM Forward Step      │  │
│                                      │    Hysteresis EMA layer  │  │
│                                      └──────────┬───────────────┘  │
│                                                 │ Tauri IPC         │
│                                      ┌──────────▼───────────────┐  │
│                                      │   React/TS Frontend      │  │
│                                      │   Dashboard + Overlay    │  │
│                                      │   Lv1 Nudge (red mist)   │  │
│                                      │   Lv2 Wall  (full block) │  │
│                                      └──────────┬───────────────┘  │
│                                                 │ unlock            │
│                                      ┌──────────┴───────────────┐  │
│                                      │  Accelerometer (WinRT)   │  │
│                                      │  Physical motion → unlock │  │
│                                      └──────────────────────────┘  │
│                                                 │                   │
│                                      ┌──────────▼───────────────┐  │
│                                      │   SessionLogger (Rust)   │  │
│                                      │   NDJSON → Documents/    │  │
│                                      │   GSE-sessions/          │  │
│                                      └──────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────┘
```

### Thread Model

```
Main Thread (Tauri event loop)
    │
    ├─ Hook Thread            ← WH_KEYBOARD_LL message loop + WinEvent IME callbacks
    │       │ crossbeam::channel (bounded 64, non-blocking send)
    │       │ bounded(1) wake channel → IME Polling Thread
    ├─ Analysis Thread        ← recv_timeout(1 s) → 1 Hz timer gate; HMM update ≤ 1×/sec
    │       │ Arc<Mutex<CognitiveStateEngine>> (Tauri managed state)
    │       │ Synthetic Friction: silence ≥ 20 s → ramps F6/F3 toward Stuck
    ├─ IME Monitor Thread     ← polls is_candidate_window_open() every 100 ms (composition pause)
    ├─ IME Polling Thread     ← recv_timeout(100 ms) wake; ImmGetOpenStatus + VK_DBE_* fallback
    │
    ├─ Logger Thread          ← bounded channel(512) → NDJSON file (BufWriter)
    └─ Sensor Thread          ← Accelerometer (WinRT) → Wall unlock event
```

---

## Folder Structure

```
GSE-Next/
├── analysis/
│   ├── behavioral_gt.py       # Post-session behavioral GT labeling
│   └── hmm_sensitivity.py     # Parameter sensitivity analysis
│
├── src/                       # React / TypeScript frontend
│   ├── components/
│   │   ├── Dashboard.tsx      # State probability bars + session info
│   │   └── Overlay.tsx        # Nudge (red mist) + Wall (full-screen block)
│   ├── App.tsx                # Intervention state machine (Lv1 → Lv2 escalation)
│   └── main.tsx
│
├── src-tauri/                 # Rust / Tauri 2.0 backend
│   ├── capabilities/
│   │   └── default.json       # Tauri 2.0 capability declarations
│   ├── src/
│   │   ├── analysis/
│   │   │   ├── engine.rs      # HMM + hysteresis layer (display_probs EMA)
│   │   │   ├── features.rs    # F1–F6 extraction + silence synthesis
│   │   │   └── mod.rs
│   │   ├── input/
│   │   │   ├── hook.rs        # WH_KEYBOARD_LL hook + WinEvent IME detection
│   │   │   ├── ime.rs         # ImeMonitor (EnumWindows + UIAutomation fallback)
│   │   │   └── mod.rs
│   │   ├── lib.rs             # Tauri setup, thread orchestration, IPC commands
│   │   ├── logger.rs          # Async NDJSON session logger
│   │   ├── main.rs
│   │   └── sensors.rs         # Accelerometer + Geolocator (WinRT sensors)
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
This form is analogous to a one-sided z-score with an implicit σ = κ·β.

---

## HMM Engine

### Semantic Latent Axes: Cognitive Friction × Productive Silence

The six normalized features are projected onto two interpretable semantic latent axes before discretization:

```text
X (Cognitive Friction)   = 0.30·φ(F3) + 0.25·φ(F6) + 0.25·φ(F1) + 0.20·φ(F5)
Y (Productive Silence)   = 0.40·φ(F4) + 0.35·(1 − φ(F1)) + 0.25·(1 − φ(F5))
```

**Cognitive Friction ($X$)**: Quantifies the depth of "hesitation" or struggle, heavily weighting the Stuck index $P(\text{Pause} \mid \text{Delete})$ (represented by F6).

**Productive Silence ($Y$)**: Indicates how much a silence leads to a productive burst. This separates valuable DMN-activated incubation from mere cognitive stalling.

Both axes are smoothed with an Exponential Weighted Moving Average (α = 0.30) to suppress single-keystroke noise:
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
- **ε** = 0.04 (emission floor, prevents probability absorption, reduces clustering)

### Transition Matrix A

| From \ To | Flow | Incubation | Stuck |
|---|---|---|---|
| **Flow** | 0.80 | 0.13 | 0.07 |
| **Incubation** | 0.12 | 0.80 | 0.08 |
| **Stuck** | 0.06 | 0.18 | 0.76 |

Flow self-transition 0.80 → mean dwell ≈ 5 s.
Incubation 0.80 → consistent with Sio & Ormerod (2009): incubation typically lasts seconds to minutes.
Stuck 0.76 → consistent with high perseveration tendency (Ohlsson, 1992).

---

## Hysteresis & Stability Fixes (v2.1)

Three pathological behaviors were identified from session log analysis and corrected:

---

### Fix ①: Cold-Start Hysteresis (Stuck → Flow window-reset spike)

**Theoretical Note: O(1) Alternative to HSMM**
Standard HMMs cannot model state duration distributions explicitly (they assume geometric decay). While a Hidden Semi-Markov Model (HSMM) is theoretically optimal for modeling the distinct, non-geometric durations of Incubation and Stuck, it introduces $O(T^2)$ computational complexity and requires massive data to estimate duration parameters (overfitting in $n=1$ environments). The EMA hysteresis layer introduced below acts as an $O(1)$ computational hack to enforce minimum state dwell times without the overhead of an HSMM, making it ideal for edge inference.

**Problem:** At t = 255.2 s a 30-second window advanced past heavy backspacing activity. The deleted events exited the window; fresh-window features looked like Flow. Result: `p_stuck = 0.994 → p_flow = 0.48` in one HMM step (< 1 ms).

**Root cause:** `get_current_state()` returned the raw HMM belief, which is a point estimate with no temporal inertia.

**Fix:** A secondary probability vector `display_probs` is maintained alongside the raw HMM belief. It tracks the raw belief via a slow EMA:

```
display_t = α · raw_t + (1 − α) · display_{t−1}

α = 0.25  (normal updates → time-constant τ ≈ 4 updates ≈ 4 s)
α = 0.50  (backspace-penalty bin → rapid Stuck onset)
```

`get_current_state()` returns `display_probs`. A genuine state change now requires approximately 4 seconds of sustained evidence to register in the UI and log.

Simulation of the window-reset scenario:

| Tick | raw p_flow | display p_stuck |
|---|---|---|
| 0 (before reset) | 0.01 | **0.994** |
| 1 (reset, flow signal) | 0.48 | 0.748 |
| 2 | 0.52 | 0.563 |
| 3 | 0.54 | 0.424 |
| 4 | 0.56 | 0.319 |

The Stuck display probability decays gracefully over ~4 seconds rather than collapsing instantly.

---

### Fix ②: Probability Discrete Clustering (step-wise ceilings)

**Problem:** With emission floor ε = 0.01 the HMM converged to fixed-point probabilities:
- `p_flow` clustering at 0.9734 (39.1% of frames)
- `p_inc` clustering at 0.9381
- `p_stuck` clustering at 0.9944

These clusters arise because for observation bins dominated by one state, the ratio of emission probabilities determines a unique fixed point. With ε = 0.01 the ratios are extreme (e.g. 0.20 : 0.01 : 0.01 = 20:1:1), concentrating almost all probability mass.

**Fix:** Emission floor raised from 0.01 to **0.04**. This adds equal additive smoothing to all state likelihoods for a given observation, effectively applying Laplace (additive) smoothing in the emission domain. Maximum attainable probability per state now saturates near 0.88–0.90, leaving meaningful probability mass for competing states and enabling smoother probability trajectories.

---

### Fix ③: Inc → Stuck Silence Transition (50 s no-input stays Inc)

**Problem:** Long silences (≥ 50 s) were classified as Incubation because `make_silence_observation()` only populated F5 (pause count). The maximum X (Friction) achievable from F5 alone is:

```
X_max(F5 only) = 0.20 · φ(F5) = 0.20 · 1.0 = 0.20   → x_bin = 1 (Incubation)
```

The Stuck-dominant bins require x_bin ≥ 3 (X ≥ 0.60), which was unreachable without F3 or F6.

**Cognitive rationale:** Prolonged output silence is not semantically equivalent to deliberate incubation. When a writer stares at a screen without typing for over 30 seconds, the behavioral interpretation shifts from "thinking" toward "blocked." Synthesizing friction during extreme silence reflects this cognitive transition.

**Fix:** `make_silence_observation()` now generates synthetic friction values that increase linearly with silence duration:

```rust
// F6 onset at 20 s → reaches 0.50 at 80 s
F6_synthetic = clamp((silence_secs − 20) / 60,  0.0, 0.50)

// F3 onset at 30 s → reaches 0.40 at 130 s
F3_synthetic = clamp((silence_secs − 30) / 100, 0.0, 0.40)
```

Resulting X trajectory (with F5 saturated at φ = 1.0, typical F1):

| Silence | F3_syn | F6_syn | X (Friction) | x_bin | Region |
|---|---|---|---|---|---|
| 20 s | 0.00 | 0.00 | ≈ 0.20 | 1 | Incubation |
| 30 s | 0.00 | 0.17 | ≈ 0.30 | 1 | Incubation |
| 40 s | 0.10 | 0.33 | ≈ 0.52 | 2 | Boundary |
| 50 s | 0.20 | 0.50 | ≈ 0.75 | 3 | **Stuck** |

After EWMA smoothing (α = 0.30), the Stuck observation registers over approximately 5 additional seconds. Combined with the hysteresis layer, the Stuck label is confirmed after ~9 s of sustained high-friction silence.

---

## IME Detection and Context-Aware Baseline (v2.2)

Japanese IME input introduces two distinct challenges for keystroke analysis:

1. **Candidate selection phase** (space → kanji list visible): navigation keystrokes (arrow, Enter) should not pollute feature vectors
2. **Input mode context** (あ mode vs A mode): flight-time norms differ significantly between Japanese writing and ASCII coding

### Candidate Window Suppression

When the kanji candidate list is visible, analysis is paused and the display state is held at Flow. Two detection layers are used:

| Layer | Method | Notes |
|---|---|---|
| **Primary** | `EnumWindows` scan for "CandidateUI" / "IME" window classes | Direct cross-process window scan, no hooks required |
| **Secondary** | UIAutomation `GetFocusedElement` | Fallback when focused element is the IME candidate panel |

Global TSF hooks (`ITfThreadMgr`) are not used — they are blocked by UIPI across process boundaries.

**MSCTFIME UI is explicitly excluded:** This class belongs to the TSF language bar (the A/あ indicator on the taskbar), which is always visible when Japanese IME is loaded. Including it would cause a permanent false positive.

### IME Mode Detection via VK\_DBE\_\* Keys (v2.2)

**Why `VK_PROCESSKEY` does not work:** `WH_KEYBOARD_LL` fires before the IME engine processes input. Keys typed during composition therefore arrive as their raw codes (`VK_A`, `VK_I`, etc.), not as `VK_PROCESSKEY` (0xE5). This is by design — the low-level hook is below the IME layer.

**What does work:** Windows delivers IME mode-switch keys — `VK_DBE_ALPHANUMERIC` (0xF0), `VK_DBE_HIRAGANA` (0xF2), `VK_DBE_KATAKANA` (0xF1), etc. — to `WH_KEYBOARD_LL` before IME processing, because these are hardware-level input mode toggles. **Empirical confirmation:** `VK_DBE_ALPHANUMERIC` (vk = 240) was observed 14 times in session log `gse_20260222_074741.ndjson`, proving that mode-switch keys reach the hook reliably.

The hook thread maintains `IME_OPEN: AtomicBool`:

| VK Code | Value | Effect on `IME_OPEN` |
|---|---|---|
| `VK_DBE_ALPHANUMERIC` (0xF0) | Switch to alphanumeric (A) | → `false` |
| `VK_DBE_SBCSCHAR` (0xF3) | Single-byte mode | → `false` |
| `VK_DBE_HIRAGANA` (0xF2) | Hiragana mode (あ) | → `true` |
| `VK_DBE_KATAKANA` (0xF1) | Katakana mode (カ) | → `true` |
| `VK_DBE_DBCSCHAR` (0xF4) | Double-byte mode | → `true` |
| `VK_KANJI` (0x19) | Half/full-width toggle | → flip |

**Limitation:** The initial state is assumed `false` (alphanumeric). The flag becomes accurate after the first mode-switch key is pressed.

### Context-Specific Baseline β (Dual Baseline)

The `φ(x, β)` normalization function uses different reference values depending on the current IME mode, correcting a systematic bias:

| Context | `IME_OPEN` | β\_F1 (ms) | β\_F3 | β\_F4 | β\_F5 | β\_F6 |
| --- | --- | --- | --- | --- | --- | --- |
| **β\_coding** (A mode) | `false` | 150 | 0.06 | 5.0 | 2.0 | 0.08 |
| **β\_writing** (あ/カ mode) | `true` | 220 | 0.08 | 2.0 | 4.0 | 0.12 |

**Why this matters:** A coder at 150 ms flight time appears as Flow against β\_coding but would incorrectly appear as Stuck if evaluated against the slower β\_writing baseline (and vice versa). Context switching eliminates this systematic misclassification.

IME mode changes are logged as a new record type:

```jsonc
// IME mode switch: user pressed VK_DBE_HIRAGANA (→ Japanese writing mode)
{"type":"ime_state","t":1740000001234,"on":true}

// IME mode switch: user pressed VK_DBE_ALPHANUMERIC (→ alphanumeric/coding mode)
{"type":"ime_state","t":1740000005678,"on":false}
```

---

## IME Mode Detection: Robust Multi-Layer Architecture (v2.3)

Version 2.2 described the initial VK\_DBE\_\* based IME mode detection. Empirical testing on a Surface Pro 8 Type Cover revealed three additional failure modes that required a complete rework of the detection pipeline.

### Bugs Found and Fixed

**Bug 1 — Same-millisecond flapping** (log `gse_20260225_142614.ndjson`):
The analysis thread emitted `ImeState` entries at the exact same millisecond as the triggering keystroke, producing `on:true` + `on:false` pairs with identical timestamps. Root cause: the analysis thread computed IME state synchronously from the keystroke event timestamp.

**Bug 2 — Zero detections via `ImmGetContext`** (log `gse_20260225_153634.ndjson`):
A pure polling approach using `ImmGetContext(foreground_hwnd)` returned NULL for all cross-process windows on Windows 10/11. The system silently produced zero `ime_state` entries.

**Bug 3 — Surface Type Cover keyboard asymmetry** (log `gse_20260225_162114.ndjson`):
On this specific keyboard, `VK_DBE_HIRAGANA` (0xF2) fires **only as key-UP** and `VK_DBE_ALPHANUMERIC` (0xF0) fires **only as key-DOWN**, always paired within the same millisecond. The `is_press`-only handler in v2.2 missed the hiragana key entirely.

**Bug 4 — Initial state never emitted**:
With `last_state: bool = false` and `IME_OPEN` initialized to `false`, pressing `VK_DBE_ALPHANUMERIC` set `IME_OPEN = false` (no change), making `false != false` evaluate to `false` — no emission.

### v2.3 Architecture: Three Detection Layers

```
Layer 1 (Primary): ImmGetOpenStatus polling
  └─ Works when ImmGetContext returns non-NULL (rare: own-process windows only)

Layer 2 (Fallback): VK_DBE_* atomic tracking via IME_STATE_DIRTY flag
  ├─ hook_callback sets IME_OPEN + IME_STATE_DIRTY on both key-DOWN and key-UP
  └─ Covers Surface Type Cover asymmetry (VK_DBE_HIRAGANA key-UP only)

Layer 3 (Inference): WinEvent EVENT_OBJECT_IME_CHANGE/SHOW
  └─ Composition can ONLY start in Japanese mode → infer IME_OPEN=true cross-process
```

### Anti-Flapping Guarantee

`LogEntry::ImeState` is emitted **exclusively** from the IME Polling Thread, never from the keyboard hook or analysis thread. The flow is:

```
hook_callback (key-DOWN or key-UP)
    │  atomic store: IME_OPEN = new_state
    │  atomic store: IME_STATE_DIRTY = true
    └─ try_send(()) on bounded(1) wake channel (non-blocking, drops if full)

IME Polling Thread
    │  recv_timeout(100ms)   ← wakes within 1ms of keypress signal
    │  sleep(5ms)            ← allows IME engine to settle
    │  read IME_STATE_DIRTY  ← check if state changed
    │  emit LogEntry::ImeState { timestamp: SystemTime::now(), on }
    └─ timestamp is always ≥5ms after the triggering keystroke
```

### Key Atomics (`hook.rs`)

| Atomic | Writer | Reader | Purpose |
|---|---|---|---|
| `IME_OPEN` | hook (VK_DBE_* ↑↓, WinEvent), polling thread | analysis thread | Current IME mode |
| `IME_STATE_DIRTY` | hook (VK_DBE_* ↑↓, WinEvent composition start) | polling thread (`.swap(false)`) | Triggers log emission |
| `IME_ACTIVE` | WinEvent hook (composition start/end) | ImeMonitor (100ms poll) | Candidate window pause |

### `Option<bool>` Initial State

The polling thread tracks `last_state: Option<bool> = None` (not `bool = false`). The comparison:

```rust
if last_state.map_or(true, |prev| prev != current) { emit(); last_state = Some(current); }
```

ensures the **first detection always emits** regardless of whether the initial state is `false` or `true`.

### Production Verification

Log `gse_20260225_172338.ndjson` confirms the fix:

| Observation | Result |
|---|---|
| `ime_state` entries detected | **9** (vs. 0 in v2.2) |
| Same-timestamp flapping | **None** |
| Minimum key → ime\_state lag | **5 ms** (VK\_DBE\_* path) |
| WinEvent composition inference lag | **~23–38 ms** |
| Alternating on/false pattern | **Correct** |

---

## 1Hz Timer-Driven Inference & Synthetic Friction (v2.4)

### Decoupling from Keystroke Events

In the initial implementation, the HMM was updated synchronously on every keystroke. This caused the EMA hysteresis layer (τ ≈ 4 seconds) to behave inconsistently: during fast typing (10+ keys/sec), the α = 0.25 update accumulated far more than once per second, while during silence the system received no updates at all.

**Fix:** The analysis thread now enforces a **1 Hz timer gate**. The `crossbeam::channel::recv_timeout(1000 ms)` call naturally provides a 1-second tick:

- **Keystroke arrives within 1 s:** Event is buffered; features are extracted and accumulated, but `engine.update()` is called only if ≥ 1 second has elapsed since the last HMM step.
- **Timeout (no input for 1 s):** A synthetic silence observation is generated via `make_silence_observation()` and fed to the HMM.

This decoupling ensures exactly **one HMM forward step per second**, making the EMA time constant τ = 1/α ≈ 4 updates = **4 seconds** mathematically precise regardless of typing speed.

### Synthetic Friction

When the 1 Hz timer fires during extended silence, `make_silence_observation()` generates **synthetic friction** — artificial F3 (correction rate) and F6 (pause-after-delete rate) values that ramp up linearly with silence duration. This mechanism ensures that prolonged inactivity (≥ 50 s) transitions the HMM from Incubation to Stuck, reflecting the cognitive shift from deliberate pause to unproductive stalling. See **Fix ③** in the Hysteresis section above for the detailed derivation and trajectory table.

---

## Intervention UI: Nudge & Wall (v2.5)

When the HMM detects a Stuck state, the system provides graduated intervention through Tauri's transparent multi-window overlay system.

### Two-Level Intervention

| Level | Name | Trigger | Visual Effect | User Interaction |
| --- | --- | --- | --- | --- |
| **Lv1** | Nudge | p\_stuck > 0.60 | Red vignette (mist) around screen edges | Click-through (transparent to input) |
| **Lv2** | Wall | p\_stuck > 0.70 for 30 s | Full-screen blocking overlay with message | Blocks all input until unlocked |

### Nudge (Lv1): Ambient Friction Cue

The Nudge layer renders a semi-transparent red vignette overlay whose opacity scales linearly with the Stuck probability:

```text
opacity = clamp((p_stuck − 0.60) / 0.30, 0.0, 1.0)
```

The overlay window is set to **click-through** mode via Tauri's `setIgnoreCursorEvents(true)`, allowing the user to continue working while receiving a peripheral visual cue that they may be stuck.

### Wall (Lv2): Forced Break

If p\_stuck remains above 0.70 for 30 consecutive seconds, the system escalates to a Wall — a full-screen blocking overlay that requires **physical movement** to dismiss. The overlay displays:

> *"Time to Move! Please stand up and walk around to unlock."*

The Wall is unlocked by the **accelerometer** (via WinRT `Windows.Devices.Sensors.Accelerometer`). When sufficient physical motion is detected, a `"sensor-accelerometer"` event with `"move"` payload triggers the Wall to dismiss and reset the intervention timer.

### Design Rationale

This graduated intervention is grounded in the insight that cognitive blocks often persist because the writer lacks awareness of being stuck (metacognitive failure). The Nudge provides gentle awareness without disruption; the Wall enforces a physical context switch, which research suggests facilitates problem restructuring (Sio & Ormerod, 2009).

---

## Logging & Analysis

Every session produces a timestamped NDJSON file:

```
%USERPROFILE%\Documents\GSE-sessions\gse_YYYYMMDD_HHMMSS.ndjson
```

Record types:

```jsonc
// Session metadata
{"type":"meta","session_start":1740000000000}

// Raw keystroke event
{"type":"key","t":1740000001234,"vk":65,"press":true}

// Feature snapshot + HMM state probabilities (after each key press or silence tick)
{"type":"feat","t":1740000001235,
 "f1":145.20,"f2":312.00,"f3":0.0800,"f4":6.50,"f5":1.0,"f6":0.0000,
 "p_flow":0.7123,"p_inc":0.2100,"p_stuck":0.0777}

{"type":"meta","session_end":1740000060000}
```

### Post-session Ground-Truth Labeling

```bash
python analysis/behavioral_gt.py gse_YYYYMMDD_HHMMSS.ndjson
```

Labels are assigned per 30-second sliding window (1-second step):

| Label | Behavioral Rule |
|---|---|
| **FLOW** | median(FT) < 200 ms AND correction\_rate < 0.15 AND not STUCK/INC |
| **INCUBATION** | Pause(≥ 2 s) → Burst(≥ 5 chars at FT < 200 ms) → diff\_chars ≥ 3 within 30 s |
| **STUCK** | ≥ 3× (Burst(≤ 3 chars) → Delete(≥ 1) → Pause(≥ 2 s)) in 60 s AND diff\_chars ≤ 0 |
| **UNKNOWN** | No condition met, or multiple labels conflict |

---

## Build Instructions

### Prerequisites

| Tool | Version |
|---|---|
| Rust | 1.77+ (`rustup update stable`) |
| Node.js | 20+ |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |

### Development

```bash
cd GSE-Next
npm install
npm run tauri dev
```

### Release Build

```bash
npm run tauri build
# Installer: src-tauri/target/release/bundle/
```

### Post-session Analysis

```bash
python analysis/behavioral_gt.py "%USERPROFILE%\Documents\GSE-sessions\gse_YYYYMMDD_HHMMSS.ndjson"
```

---

## Academic References

1. **Csikszentmihalyi, M.** (1990). *Flow: The Psychology of Optimal Experience*. Harper & Row.
   — Foundation for the Flow cognitive state definition and its behavioral correlates.

2. **Csikszentmihalyi, M.** (1996). *Creativity: Flow and the Psychology of Discovery and Invention*. HarperCollins.
   — Extends flow theory to creative and generative writing tasks.

3. **Sio, U. N., & Ormerod, T. C.** (2009). Does incubation enhance problem solving? A meta-analytic review. *Psychological Bulletin, 135*(1), 94–120.
   — Empirical basis for the Incubation state self-transition probability (0.80) and the Pause→Burst behavioral signature.

4. **Ohlsson, S.** (1992). Information-processing explanations of insight and related phenomena. In M. T. Keane & K. J. Gilhooly (Eds.), *Advances in the Psychology of Thinking* (pp. 1–44). Harvester Wheatsheaf.
   — Impasse and perseveration theory underlying the Stuck state model and its high self-transition probability.

5. **Rabiner, L. R.** (1989). A tutorial on hidden Markov models and selected applications in speech recognition. *Proceedings of the IEEE, 77*(2), 257–286.
   — HMM Forward Algorithm formulation used in `CognitiveStateEngine::update()`.

6. **Dhakal, V., Feit, A. M., Kristensson, P. O., & Oulasvirta, A.** (2018). Observations on typing from 136 million keystrokes. *Proceedings of CHI 2018*.
   — Population baseline values for flight time and correction rate used in the φ normalization reference values (β).

7. **Salthouse, T. A.** (1986). Perceptual, cognitive, and motoric aspects of transcription typing. *Psychological Bulletin, 99*(3), 303–319.
   — Flight-time decomposition and anticipatory processing in skilled typing; informs F1 and F4 feature design.

8. **Microsoft Corporation.** (2023). *WinEvent Hooks*. Windows Developer Documentation (MSDN).
   — `SetWinEventHook`, `EVENT_OBJECT_IME_CHANGE/SHOW/HIDE` constants, and `WINEVENT_OUTOFCONTEXT` flag; used for cross-process IME detection without DLL injection.

---

## License

Research prototype. All rights reserved.

---

*Last updated: 2026-02-27*
