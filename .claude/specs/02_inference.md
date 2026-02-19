Layer 2: Inference Engine (MhD Core) Specification
Objective
Estimate cognitive state (FLOW, INCUBATION, STUCK) from keystroke features (F1-F6).

Architecture: Hybrid Approach
We use a staged approach. Start with Stage 1, then prepare for Stage 4.

Stage 1: Heuristic & HMM (Rust Native)
Library: Use karma crate for Hidden Markov Models.

States: 3 Hidden States (Flow, Incubation, Stuck).

Observations: Discretized feature vectors from Layer 1.

Logic:

Update state probability every 1 second.

If P(Stuck) > 0.7 for 5 seconds -> Trigger Intervention.

Stage 4: Deep Learning (ONNX)
Library: Use ort crate (ONNX Runtime bindings).

Model: Pre-trained Bi-LSTM model (exported from PyTorch as .onnx).

Input: Sequence of (Key, Duration, FlightTime) x 30 events.

Execution: Run inference in a background tokio task to avoid freezing the UI.

Requirements
Define a StateEstimator trait to easily swap between Rule-based, HMM, and ONNX models.

Performance: Inference must complete within 5ms.

Let's think step by step.