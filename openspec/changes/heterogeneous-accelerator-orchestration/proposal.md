# Proposal: Change 044 - Full-Spectrum Heterogeneous Orchestration

## Context
A modern PaaS must offer "Hardware Affinity". Not all AI models belong on a GPU. 
- **GPU:** Large batch LLM inference (VRAM bound).
- **NPU:** Real-time audio/vision (latency/power bound).
- **TPU:** Massive matrix multiplications and training-style inference (Google/Coral hardware).
- **CPU:** Small models (BERT, classic ML) or fallbacks where precision/branching logic is complex.

## Objective
1. Implement a Universal Backend Router for WASI-NN.
2. Abstract hardware drivers behind a unified `BackendTrait`.
3. Allow the `integrity.lock` to define strict hardware mapping.
4. Support asynchronous multi-device execution (parallelism across 4+ hardware queues).

## Success Metrics
- A single Tachyon node can simultaneously run 4 different models on 4 different hardware types.
- The Rust host remains stable and thin by dynamically linking to specialized drivers (CUDA, OpenVINO, LibTPU).
- Zero interference: a heavy TPU task does not slow down a real-time NPU audio task.