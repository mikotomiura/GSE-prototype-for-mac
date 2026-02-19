# GSE (Generative Struggle Engine) Prototype v2

[🇯🇵 Japanese (日本語)](./README.ja.md) | [🇺🇸 English](./README.md)

GSE is an experimental system designed to monitor, visualize, and optimize a user's **Internal Cognitive State** during creative tasks (e.g., coding, writing). By analyzing keystroke dynamics in real-time, it infers whether the user is in a state of **Flow** (productivity), **Incubation** (thoughtful pause), or **Stuck** (frustration/blockage) and provides subtle visual feedback to guide them back to an optimal state.

## 🌟 Key Features

### 1. Real-time Cognitive State Inference
- **Hidden Markov Model (HMM)**: Uses a probabilistic model to infer invisible cognitive states based on observable keystroke flight times.
- **Micro-Behavior Analysis**:
  - **Typing Rhythm**: Fast, consistent typing suggests **Flow**.
  - **Pauses**:
    - Brief pauses (1.5s - 5.0s) suggest **Stuck** (hesitation).
    - Long pauses (> 5.0s) suggest **Incubation** (idle/break).
  - **Editing Patterns**:
    - Frequent **Backspace** usage (especially >3 consecutive presses) strongly indicates **Stuck** (rethinking/correction).
    - **Space/Conversion** during IME input is treated as **Flow** (generative).

### 2. Ambient Visual Feedback
- **Dashboard Widget**: A minimal, always-on-top widget displaying real-time probability gauges for Flow, Incubation, and Stuck.
- **Mist Effect**: When the user remains **Stuck** for a prolonged period (approx. 30s), a subtle "mist" overlay gently fades in to signal the need for a change in approach or a break.

### 3. System Compatibility
- **Global Key Hook**: Captures input across all applications (system-wide) using Windows APIs (`WH_KEYBOARD_LL`).
- **IME Awareness**: Detects when an Input Method Editor (IME) candidate window is open to prevent false "Stuck" positives during language conversion.
- **Low Overhead**: Optimized Rust backend ensures minimal impact on system performance.

---

## 🛠️ Technology Stack

- **Frontend**: React, TypeScript, Vite
  - Handles UI visualization and animations.
  - Communicates with the backend via Tauri commands.
- **Backend**: Rust, Tauri v2
  - **Core Logic**: `CognitiveStateEngine` (HMM implementation).
  - **System Interaction**: Windows API (Win32) for hooks and IME detection.
- **OS**: Windows 10/11

---

## 📂 File Structure

```
GSE-Next/
├── src/
│   ├── components/
│   │   └── Dashboard.tsx       # Main UI widget and Mist effect logic
│   ├── App.tsx                 # Root component handling state updates
│   ├── App.css                 # Styling for dashboard and animations
│   └── main.tsx                # Entry point
├── src-tauri/
│   ├── src/
│   │   ├── analysis/
│   │   │   ├── engine.rs       # HMM Inference Engine & State Logic
│   │   │   └── features.rs     # Feature extraction (Flight Time, EMA)
│   │   ├── input/
│   │   │   ├── hook.rs         # Low-level Keyboard Hook (WH_KEYBOARD_LL)
│   │   │   └── ime.rs          # IME Candidate Window Monitor
│   │   └── lib.rs              # Main library entry & command registration
│   ├── tauri.conf.json         # Tauri configuration (permissions, windows)
│   └── Cargo.toml              # Rust dependencies
└── index.html                  # HTML entry point
```

---

## 🧠 Technical Details: The Inference Engine

The core of GSE is the **Cognitive State Engine** (`engine.rs`), which implements a Hidden Markov Model.

### States
1.  **Flow**: High-speed, rhythmic input.
2.  **Incubation**: Strategic pauses, idle time.
3.  **Stuck**: Irregular rhythm, frequent corrections, short hesitation.

78: ### Logic Flow
79: 1.  **Input Capture**: `hook.rs` intercepts key press/release events globally.
80: 2.  **Feature Extraction**: `features.rs` now calculates **6 Micro-Features (F1-F6)** over a 30-second sliding window:
81:     - **F1 (Median Flight Time)**: Basic typing speed.
82:     - **F2 (Flight Time Variance)**: Rhythm stability.
83:     - **F3 (Correction Rate)**: Ratio of Backspace/Delete keys.
84:     - **F4 (Burst Length)**: Average length of continuous typing bursts.
85:     - **F5 (Pause Count)**: Number of pauses > 2 seconds.
86:     - **F6 (Pause After Delete)**: Pauses immediately following a correction (strong high-cognitive-load signal).
87: 3.  **HMM Update**: `engine.rs` computes a **Stuck Score (S_stuck)** based on these features and updates the HMM state probabilities.
88:     - **Transition Matrix**: Redesigned to reduce state flickering.
89:     - **Emission Matrix**: Maps `S_stuck` scores to observation bins.
90: 4.  **Stability & Quality**:
91:     - **Mutex Safety**: Replaced `unwrap()` with safe pattern matching to prevent thread panics.
92:     - **Structured Logging**: Uses `tracing` crate for detailed diagnostic logs.

---

## 🚀 Build & Run

### Prerequisites
- Node.js (v18+)
- Rust (Stable)
- Windows SDK (for native APIs)

### Commands
```bash
# Install dependencies
npm install

# Run in Development Mode
npm run tauri dev

# Build for Production
npm run tauri build
```

---

## ⚠️ Notes
- This application requires **Administrator privileges** or UI Access rights in some scenarios to hook keyboard input correctly across elevated windows.
- The "System Exit" button in the dashboard forcefully terminates the application process.
