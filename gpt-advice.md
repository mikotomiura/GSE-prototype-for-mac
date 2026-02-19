# GSE Technical Design vNext
## Keystroke-First Cognitive State Estimation System

Author: Mikoto Miura  
Target: 未踏IT人材発掘・育成事業 提出用技術設計書  
Environment Constraint: Surface Pro 8 / No GPU Required / Free Stack Only  

---

# 1. Problem Definition

## 1.1 Core Question

Can we distinguish:

- **Incubation (productive silence)**
- **Stuck (unproductive stagnation)**

using **keystroke temporal dynamics alone**?

The core hypothesis:

> The qualitative difference between "good silence" and "bad silence" lies not in pause duration, but in temporal order patterns around deletion and burst typing.

---

# 2. Formal Modeling

## 2.1 Hidden State Formulation

We model cognitive state as latent variable:

Z_t ∈ {Flow, Incubation, Stuck}

Observed keystroke features:

O_t ∈ ℝ⁶

This naturally forms a Hidden Markov Model (HMM):

P(Z_t | Z_{t-1})  → Transition Model  
P(O_t | Z_t)      → Emission Model  

---

# 3. Feature Engineering (30s Sliding Window)

## 3.1 Keystroke Features

F1: Median Flight Time  
F2: Flight Time Variance  
F3: Correction Rate  
F4: Burst Length  
F5: Pause Count (>2s)  
F6: Pause-after-Delete Rate  

Key insight:

Incubation pattern:
Pause → Burst

Stuck pattern:
Delete → Pause

Temporal ordering is critical.

---

# 4. HMM Specification

## 4.1 State Transition Matrix (Initial Prior)

Let:

Flow = 0  
Incubation = 1  
Stuck = 2  

Initial transition matrix A:

| From \ To | Flow | Incubation | Stuck |
|------------|------|------------|--------|
| Flow       | 0.7  | 0.25       | 0.05   |
| Incubation | 0.2  | 0.6        | 0.2    |
| Stuck      | 0.1  | 0.2        | 0.7    |

Design rationale:

- Flow is stable
- Stuck is sticky
- Incubation is transitional

These are priors; refined using Baum-Welch if data sufficient.

---

## 4.2 Emission Model

Gaussian emission:

O_t ~ N(μ_k, Σ_k)

Where k ∈ {Flow, Incubation, Stuck}

Initial μ_k derived from rule-based clustering.

---

## 4.3 Inference

Online decoding using:

- Viterbi Algorithm (for most probable state sequence)
- Forward probability (for confidence score)

Computation complexity:
O(T * N²)

With N=3, real-time CPU feasible.

---

# 5. Calibration Strategy

First launch:

5-minute free typing session.

User baseline:

Baseline_FlightTime  
Baseline_BurstLength  

All features normalized:

F'_i = F_i / Baseline_i

Reduces inter-user variance.

---

# 6. Intervention Logic

Intervention triggered only when:

P(Stuck | O_{1:t}) > 0.8  
AND sustained > 5s  

Levels:

1. Nudge (visual border)
2. Ambient Fade
3. Thought Scaffold
4. The Wall (Monk Mode only)

Safety:

- Ctrl+Shift+Esc override
- Max lock 5 min

---

# 7. Prototype Completion Definition (3/11 Target)

A "completed prototype" means:

✓ Real-time keystroke logging  
✓ Feature extraction  
✓ HMM inference (Viterbi working)  
✓ State visualization panel  
✓ STUCK-triggered Nudge  
✓ Log export (CSV)  

Not required:

✗ EEG integration  
✗ LSTM training  
✗ GPU usage  

---

# 8. Evaluation Plan (Minimal but Sufficient)

## 8.1 Dataset

Self-collected:
1 hour coding/writing session

Manual labeling:

Flow / Incubation / Stuck

## 8.2 Metrics

- Confusion Matrix
- Accuracy
- F1-score (Stuck class)

Even 65–75% accuracy is acceptable as proof-of-concept.

---

# 9. Novelty Claim (Precisely Scoped)

Not:

- “World’s first cognitive OS”

But:

- First attempt to distinguish Incubation vs Stuck using temporal deletion-pause ordering patterns in keystroke dynamics.
- First Japanese IME-aware cognitive state modeling (Phase 2 extension).

---

# 10. Technical Risk Assessment

Risk 1: Overfitting to self-data  
Mitigation: Relative normalization + rule-based fallback

Risk 2: HMM instability  
Mitigation: Fixed transition priors + smoothing

Risk 3: False positives in intervention  
Mitigation: High probability threshold + hysteresis

---

# 11. Implementation Stack (Free / Surface Pro 8 Compatible)

Frontend: Tauri (Rust)  
Backend: Rust + HMM crate or custom implementation  
Inference: Pure CPU  
Logging: CSV  
Visualization: Lightweight overlay  

No paid service required.

---

# 12. Strategic Positioning for 未踏

This project is positioned as:

- A computational modeling challenge
- A human-computer interaction redefinition
- A measurable engineering problem

Not as philosophy.
Not as AGI.

But as:

"A real-time latent cognitive state estimation system using keystroke dynamics."

---

# Final Statement

The goal of this prototype is not perfection.

It is to demonstrate:

1. Latent state modeling feasibility
2. Real-time inference capability
3. Measurable distinction between Incubation and Stuck

Under strict hardware and resource constraints.

If successful, it establishes a new direction in keystroke-based cognitive computing.
