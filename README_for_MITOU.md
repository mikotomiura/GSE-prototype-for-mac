# GSE-Next — Generative Struggle Engine: Technical Review Guide for MITOU Evaluators

> **Real-time cognitive state estimation (Flow / Incubation / Stuck) from keystroke dynamics on Windows 11.**
> No cloud dependency. No wearables. No user interruption.

This document is written for MITOU IT project evaluators (PMs) who want to quickly locate and understand the most technically substantive parts of this prototype. It explains *what* the code does, *why* the specific engineering choices were made, and *where* to find the relevant implementation.

---

## 1. Project Overview

GSE (Generative Struggle Engine) estimates a writer's real-time cognitive state from the timing of their keystrokes alone. The three states — **Flow**, **Incubation**, and **Stuck** — are grounded in cognitive science literature (Csikszentmihalyi 1990, Sio & Ormerod 2009, Ohlsson 1992) and mapped to measurable behavioral signatures:

| State | Behavioral Signature |
|---|---|
| **Flow** | Short inter-key intervals, low error rate, long continuous bursts |
| **Incubation** | Extended silence (≥2 s) followed by a rapid-output burst |
| **Stuck** | Perseverative delete–pause loops with near-zero net character gain |

The system runs as a transparent floating overlay on Windows 11, requiring **zero hardware beyond the keyboard already present**. It uses a hand-calibrated Hidden Markov Model (HMM) operating on six engineered keystroke features.

---

## 2. System Architecture

### 2.1 Technology Stack

| Layer | Technology | Rationale |
|---|---|---|
| Frontend | React 18 + TypeScript + Vite | Lightweight overlay UI; Tauri IPC for state streaming |
| Backend | Rust + Tauri 2.0 | Zero-cost abstractions for real-time signal processing |
| OS Integration | Windows API (`windows` crate v0.58) | Low-level keyboard hook; IME detection without DLL injection |
| Inference | Manual HMM (no ONNX at this stage) | Fully auditable; no runtime dependency |
| Logging | NDJSON (newline-delimited JSON) | Post-session analysis in Python |

### 2.2 Thread Architecture

```
Main Thread (Tauri event loop)
    │
    ├─ Hook Thread            ← WH_KEYBOARD_LL global keyboard hook + WinEvent IME callbacks
    │       │ crossbeam::channel (bounded 64, non-blocking)
    │       │ bounded(1) wake signal → IME Polling Thread
    ├─ Analysis Thread        ← recv_timeout(1 s); drives HMM on keystrokes AND silence
    │       │ Arc<Mutex<CognitiveStateEngine>>  →  Tauri IPC  →  React frontend
    ├─ IME Monitor Thread     ← EnumWindows every 100 ms; pauses HMM during candidate display
    ├─ IME Polling Thread     ← wake-on-keypress; ImmGetOpenStatus + VK_DBE_* fallback
    └─ Logger Thread          ← bounded channel(512); BufWriter → NDJSON file
```

**Key design constraint:** `WH_KEYBOARD_LL` callbacks must never block — a delayed hook causes Windows to automatically uninstall it. All heavy work (HMM inference, file I/O) is offloaded to separate threads via crossbeam channels.

### 2.3 Frontend → Backend Data Flow

```
Any foreground app (e.g. VSCode, Word, browser)
    └─ keypress
         └─ WH_KEYBOARD_LL hook (hook.rs)
              └─ crossbeam::channel → Analysis Thread (lib.rs)
                   └─ FeatureExtractor::calculate_features()  (features.rs)
                        └─ CognitiveStateEngine::update()     (engine.rs)
                             └─ Tauri IPC emit("cognitive-state-update")
                                  └─ Dashboard.tsx (React)
                                       └─ probability bars + mist effect
```

---

## 3. Core Implementation: Files to Review First

### 3.1 `src-tauri/src/input/hook.rs` — OS-Level Keyboard Hook

**What it does:** Installs a system-wide `WH_KEYBOARD_LL` hook and a `WinEvent` hook for IME composition detection. This is the hardest part to get right on Windows.

**Key engineering decisions:**

```rust
// Non-blocking send — NEVER block the hook callback
if let Ok(guard) = (*EVENT_SENDER).try_lock() {
    if let Some(sender) = guard.as_ref() {
        let _ = sender.try_send(event);   // drops silently if channel full
    }
}
```

The hook callback uses `try_send` (not `send`) because blocking the `WH_KEYBOARD_LL` callback causes Windows to uninstall the hook after ~200 ms.

**IME mode detection without DLL injection:**

```rust
// VK_DBE_ALPHANUMERIC (0xF0): user switched to ASCII/coding mode
// VK_DBE_HIRAGANA    (0xF2): user switched to Japanese writing mode
// These keys reach WH_KEYBOARD_LL before the IME engine processes them.
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
        _ => {}
    }
}

// Surface Type Cover: VK_DBE_HIRAGANA fires ONLY as key-UP on this keyboard.
// Mirror handler on key-UP to cover asymmetric hardware.
if is_release {
    match vk_code {
        VK_DBE_KATAKANA | VK_DBE_HIRAGANA | VK_DBE_DBCSCHAR => {
            IME_OPEN.store(true, Ordering::Release);
            IME_STATE_DIRTY.store(true, Ordering::Release);
        }
        _ => {}
    }
}
```

**WinEvent inference for cross-process IME detection:**

```rust
// IME composition can ONLY start in Japanese mode.
// This fires cross-process without DLL injection (WINEVENT_OUTOFCONTEXT).
EVENT_OBJECT_IME_SHOW | EVENT_OBJECT_IME_CHANGE => {
    IME_ACTIVE.store(true, Ordering::Relaxed);
    IME_OPEN.store(true, Ordering::Release);   // infer: must be Japanese mode
    IME_STATE_DIRTY.store(true, Ordering::Release);
    // Wake the IME polling thread immediately
    if let Ok(guard) = (*POLL_WAKE_TX).try_lock() {
        if let Some(tx) = guard.as_ref() { let _ = tx.try_send(()); }
    }
}
```

---

### 3.2 `src-tauri/src/analysis/features.rs` — F1–F6 Feature Extraction

**What it does:** Maintains a 30-second rolling window of keystroke events and computes six features per keypress. Also generates synthetic observations during silence to keep the HMM running.

**The six features:**

```rust
pub struct Features {
    pub f1_flight_time_median:    f64,  // inter-key interval (ms), last 5 samples
    pub f2_flight_time_variance:  f64,  // timing regularity
    pub f3_correction_rate:       f64,  // (Backspace+Delete) / total keys
    pub f4_burst_length:          f64,  // avg chars in runs with gap <200ms
    pub f5_pause_count:           f64,  // count of gaps ≥2000ms in window
    pub f6_pause_after_del_rate:  f64,  // fraction of deletes followed by ≥2s gap
}
```

**F6 implementation (the most diagnostic feature for Stuck):**

```rust
// F6: Pause-after-Delete Rate
// Among all Backspace/Delete presses in the 30s window,
// what fraction were immediately followed by a ≥2s silence?
// High F6 = perseverative delete-freeze loop = Stuck signal
let del_events: Vec<usize> = ...;  // indices of BS/Del presses
let del_followed_by_pause = del_events.iter().filter(|&&i| {
    // find next key-press after this delete
    if let Some(next_press) = events[i+1..].iter().find(|e| e.is_press) {
        next_press.timestamp - events[i].timestamp >= 2000
    } else { true }  // no subsequent key = also a freeze
}).count();
f6 = del_followed_by_pause as f64 / del_events.len() as f64;
```

**Silence synthesis (Inc → Stuck transition):**

```rust
// make_silence_observation(): called every 1s when no keystrokes occur.
// Synthesizes increasing Friction as silence duration grows.
// This drives the Inc→Stuck transition after ~50s of no output.
let f6 = if silence_secs > 20.0 {
    ((silence_secs - 20.0) / 60.0).min(0.50)  // 0→0.50 from 20s to 80s
} else { 0.0 };
let f3 = if silence_secs > 30.0 {
    ((silence_secs - 30.0) / 100.0).min(0.40)  // 0→0.40 from 30s to 130s
} else { 0.0 };
```

**Normalization function φ:**

```rust
// phi(x, beta): maps raw feature to [0, 1] relative to context baseline beta
// kappa=2.0 → x=beta → phi=0.0, x=3*beta → phi=1.0
pub fn phi(x: f64, beta: f64) -> f64 {
    const KAPPA: f64 = 2.0;
    ((x - beta) / (KAPPA * beta)).clamp(0.0, 1.0)
}
```

---

### 3.3 `src-tauri/src/analysis/engine.rs` — HMM Cognitive State Engine

**What it does:** Implements a full Forward Algorithm HMM with dual-baseline normalization, EWMA smoothing on latent axes, and a hysteresis layer to prevent display flicker.

**Dual-baseline normalization (IME-context-aware):**

```rust
fn calculate_latent_axes(&self, features: &Features, ime_open: bool) -> (f64, f64) {
    // β_coding: ASCII/programming baseline (Dhakal et al. 2018 CHI)
    const BETA_CODING_F1: f64 = 150.0;   // flight time ms
    const BETA_CODING_F4: f64 = 5.0;     // burst length chars

    // β_writing: Japanese IME baseline (romaji→kana→kanji is slower)
    const BETA_WRITING_F1: f64 = 220.0;  // ~47% slower than coding
    const BETA_WRITING_F4: f64 = 2.0;    // shorter bursts (composition in segments)

    let (beta_f1, beta_f3, beta_f4, beta_f5, beta_f6) = if ime_open {
        (BETA_WRITING_F1, BETA_WRITING_F3, BETA_WRITING_F4, BETA_WRITING_F5, BETA_WRITING_F6)
    } else {
        (BETA_CODING_F1, BETA_CODING_F3, BETA_CODING_F4, BETA_CODING_F5, BETA_CODING_F6)
    };
    // Without this, a coder at 150ms would appear "Stuck" under Japanese baseline
    // and a Japanese writer at 220ms would appear "Stuck" under coding baseline.
```

**Two-axis semantic projection:**

```rust
// X (Cognitive Friction): high = struggling, blocked
// Y (Productive Engagement): high = fluent, in-flow
let x = (0.30 * phi3 + 0.25 * phi6 + 0.25 * phi1 + 0.20 * phi5).clamp(0.0, 1.0);
let y = (0.40 * phi4 + 0.35 * (1.0 - phi1) + 0.25 * (1.0 - phi5)).clamp(0.0, 1.0);
```

**HMM Forward Algorithm step (the core inference):**

```rust
// A: 3x3 transition matrix    B: 3x26 emission matrix
// π: belief vector [p_Flow, p_Incubation, p_Stuck]
//
// π'_j = (Σ_i π_i · A[i,j]) · (B[j, obs] + ε)
// π'   ← normalize(π')
const EMISSION_FLOOR: f64 = 0.04;  // prevents probability absorption

for j in 0..3 {
    let trans_sum: f64 = (0..3).map(|i| old_probs[i] * transitions[i*3+j]).sum();
    new_probs[j] = trans_sum * (emissions[j*26+obs] + EMISSION_FLOOR);
}
// ε=0.04 applies Laplace-like smoothing: max attainable p ≈ 0.88–0.90
// (prevents step-wise probability clustering at 0.97 seen with ε=0.01)
```

**Hysteresis layer (Cold-Start fix — the O(1) HSMM alternative):**

```rust
// display_probs: slow EMA tracking raw HMM belief.
// Prevents: Stuck(0.99) → Flow(0.48) in <1ms when 30s window resets.
// Instead, state decays gracefully over ~4 seconds of sustained new evidence.
let display_alpha = if apply_backspace_penalty { 0.50 } else { 0.25 };
for i in 0..3 {
    display[i] = display_alpha * new_probs[i] + (1.0 - display_alpha) * display[i];
}
// α=0.25 → time constant τ ≈ 4 updates ≈ 4 seconds
// α=0.50 → Stuck responds rapidly when ≥5 consecutive Backspaces detected
```

**Transition matrix (calibrated against cognitive science literature):**

```
Flow → [Flow=0.80, Inc=0.13, Stuck=0.07]   mean dwell ~5s
Inc  → [Flow=0.12, Inc=0.80, Stuck=0.08]   Sio & Ormerod (2009)
Stuck→ [Flow=0.06, Inc=0.18, Stuck=0.76]   Ohlsson (1992) perseveration
```

---

### 3.4 `src-tauri/src/input/ime.rs` — IME Candidate Window Detection

**What it does:** Detects when the Japanese kanji candidate list is visible (which would pollute feature vectors with navigation keystrokes) and pauses the HMM. Uses two detection layers.

```rust
pub fn is_candidate_window_open(&self) -> bool {
    // PRIMARY: WinEvent atomic flag (set by win_event_callback in hook.rs)
    let winevent_active = crate::input::hook::is_ime_active();

    // SECONDARY: EnumWindows scan for visible candidate window classes
    // "CandidateUI_UIElement" = modern Microsoft IME
    // "IME"                  = classic IMM32
    // NOTE: "MSCTFIME UI" is EXCLUDED — it's the always-visible language bar,
    //        not the candidate list. Including it caused permanent false positives.
    let enumwindows_active = is_ime_candidate_window_visible();

    // Safety: if WinEvent says active but no window found, flag is stale → reset
    if winevent_active && !enumwindows_active && !uia_active {
        crate::input::hook::IME_ACTIVE.store(false, Ordering::Relaxed);
        return false;
    }
    winevent_active || enumwindows_active || uia_active
}
```

---

### 3.5 `src-tauri/src/lib.rs` — Thread Orchestration and Tauri IPC

**What it does:** Wires all threads together, implements Tauri IPC commands, and manages the analysis loop.

**Analysis loop (silence handling + IPC):**

```rust
// Analysis Thread: processes both keystrokes and silence
loop {
    match event_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(event) => {
            extractor.process_event(event);
            if event.is_press {
                let ime_open = input::hook::IME_OPEN.load(Ordering::Relaxed);
                let features = extractor.calculate_features();
                engine.update(&features, Some(event.vk_code), ime_open);
                // log feat entry
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            // 1s silence → synthesize observation to keep HMM running
            silence_secs += 1.0;
            if let Some(features) = extractor.make_silence_observation(silence_secs) {
                engine.update(&features, None, ime_open);
            }
        }
    }
    // Emit state update to React frontend via Tauri IPC
    let state = engine.get_current_state();
    app_handle.emit("cognitive-state-update", CognitiveStatePayload { ... });
}
```

---

### 3.6 `src/components/Dashboard.tsx` — React Frontend

**What it does:** Displays real-time Flow/Incubation/Stuck probability bars and activates a screen-dimming "mist effect" when Stuck persists for 30 seconds.

```typescript
// Mist effect: activates after 30s of sustained Stuck state
React.useEffect(() => {
  let timer: number | undefined;
  if (dominant === 'Stuck') {
    timer = setTimeout(() => setMistActive(true), 30000);
  } else {
    setMistActive(false);   // clear immediately when leaving Stuck
  }
  return () => clearTimeout(timer);
}, [dominant]);
```

The window is frameless and always-on-top, drawn with transparency so it floats over any application without obstructing the work surface.

---

## 4. Key Engineering Challenges and Solutions

| Challenge | Naive Approach | Our Solution |
|---|---|---|
| Global keyboard hook without blocking | Block on event processing | `try_send` to crossbeam channel; zero-copy event dispatch |
| IME mode detection cross-process | `ImmGetContext` polling | `VK_DBE_*` key tracking + WinEvent inference + `IME_STATE_DIRTY` flag |
| Surface Type Cover asymmetry | `is_press` only | Handle both `is_press` AND `is_release` for `VK_DBE_*` |
| HMM flapping (same-ms ImeState) | Emit in hook callback | Dedicated IME polling thread; always ≥5ms after keystroke |
| Cold-Start spike (Stuck→Flow in 1ms) | Raw HMM belief in UI | `display_probs` EMA (α=0.25) — O(1) HSMM approximation |
| Long silence stays Incubation | No silence handling | `make_silence_observation()` synthesizes rising Friction |
| IME candidate navigation pollutes features | No detection | `EnumWindows` + `WinEvent` to detect candidate window |
| Coding vs Japanese baseline mismatch | Single β | Dual baseline: `β_coding` / `β_writing` selected by `IME_OPEN` |

---

## 5. NDJSON Session Log Format

Every session produces a timestamped file in `%USERPROFILE%\Documents\GSE-sessions\`:

```jsonc
{"type":"meta","session_start":1772040218273}
{"type":"ime_state","t":1772040218385,"on":false}  // IME mode: A (coding)
{"type":"key","t":1772040237362,"vk":8,"press":true}
{"type":"feat","t":1772040237362,
  "f1":145.2,"f2":312.0,"f3":0.08,"f4":6.5,"f5":1.0,"f6":0.00,
  "p_flow":0.712,"p_inc":0.210,"p_stuck":0.078}
{"type":"ime_state","t":1772040237402,"on":true}   // IME mode: あ (Japanese)
{"type":"meta","session_end":1772040412111}
```

The `ime_state` entries are emitted exclusively from the IME polling thread, guaranteeing ≥5ms separation from the keystroke that triggered the mode change (anti-flapping).

---

## 6. Build Instructions

### Prerequisites

| Tool | Version |
|---|---|
| Rust | 1.77+ (`rustup update stable`) |
| Node.js | 20+ |
| Tauri CLI v2 | `cargo install tauri-cli --version "^2"` |
| Windows | 10 or 11 (x86-64) |

### Development

```bash
cd GSE-Next
npm install
npm run tauri dev
```

### Release Build

```bash
cd GSE-Next
npm run tauri build
# Installer: src-tauri/target/release/bundle/
```

### Post-Session Analysis (Python)

```bash
python analysis/behavioral_gt.py "%USERPROFILE%\Documents\GSE-sessions\gse_YYYYMMDD_HHMMSS.ndjson"
```

---

## 7. Academic References

1. Csikszentmihalyi, M. (1990). *Flow: The Psychology of Optimal Experience*. — Flow state definition and behavioral correlates.
2. Sio, U. N., & Ormerod, T. C. (2009). Does incubation enhance problem solving? *Psychological Bulletin, 135*(1). — Incubation self-transition probability (0.80).
3. Ohlsson, S. (1992). Information-processing explanations of insight. — Stuck perseveration theory; self-transition 0.76.
4. Rabiner, L. R. (1989). A tutorial on hidden Markov models. *Proceedings of the IEEE, 77*(2). — Forward Algorithm formulation.
5. Dhakal et al. (2018). Observations on typing from 136 million keystrokes. *CHI 2018*. — Population baseline β values for φ normalization.
6. Microsoft (2023). *WinEvent Hooks*. MSDN. — `SetWinEventHook`, `EVENT_OBJECT_IME_CHANGE/SHOW/HIDE`, `WINEVENT_OUTOFCONTEXT`.

---

*Generated for MITOU IT 2026 application review. Research prototype — all rights reserved.*
