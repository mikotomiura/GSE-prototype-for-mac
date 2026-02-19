# GSE Prototype v2 (Next)

GSE (Global Sensing Engine) Next is a desktop application prototype designed to monitor and optimize the user's cognitive state using keystroke dynamics and sensor fusion.

## Features

- **Sensing Layer**: Captures keystroke flight times globally using low-level Windows hooks.
- **Inference Layer (HMM)**: Analyzes flight times to infer cognitive states:
  - 🟢 **Flow**: Optimal productivity.
  - 🟡 **Incubation**: Suggests a need for a break.
  - 🔴 **Stuck**: Suggests a need for intervention.
- **Intervention Layer**:
  - **Nudge**: Subtle visual cue (red vignette) when focus drifts.
  - **The Wall**: Full-screen overlay to force a break when "Stuck".
  - **IME Monitor**: Pauses inference during Japanese IME candidate selection to prevent false positives.
- **Sensor Fusion (Surface Pro 8)**:
  - **Accelerometer**: Detects movement to unlock "The Wall".
  - **Geolocation**: Tracks context changes.

## Tech Stack

- **Frontend**: React, TypeScript, Vite
- **Backend**: Rust, Tauri v2
- **OS**: Windows 10/11 (Required for WinRT/Win32 APIs)

## Architecture

The application uses a **Core-Shell** architecture:
- **Core (Rust)**: Handles low-level hooks, HMM inference, and sensor IO.
- **Shell (React)**: Visualizes state and renders interventions (transparent overlay).

## Development

### Prerequisites
- Node.js (v18+)
- Rust (Stable)
- Windows SDK

### Build & Run
```bash
npm install
npm run tauri dev
```
