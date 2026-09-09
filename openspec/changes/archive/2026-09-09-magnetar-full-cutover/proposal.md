# Proposal: Full Magnetar Cutover in Tachyon-Mesh

## Context & Motivation
Tachyon-Mesh currently relies on the Hugging Face `Candle` framework for local inference. While Candle provides immediate broad model support, its monolithic architecture, hardcoded memory management, and tight coupling to specific hardware paradigms conflict with our edge-native, WASM-driven vision. 

With Magnetar v0.1 reaching hardware readiness (CUDA, Reference CPU) and stabilizing its Capability-Based Scheduling and Memory Arena contracts, maintaining two parallel inference backends introduces unacceptable architectural debt. 

This proposal dictates the complete removal of Candle in favor of Magnetar as the sole inference engine.

## Assumed Strategic Regressions
To force architectural alignment across the mesh, we accept the following temporary regressions until Magnetar reaches maturity (v0.2/v0.3):

1. **Model Catalog Collapse:** Tachyon-Mesh will temporarily only support the **Qwen** model family (e.g., Qwen 1.5/3.5) via Safetensors. Llama, Mistral, and other architectures will fail to load until their specific WASM Model Components are ported to Magnetar.
2. **No Tensor Parallelism:** Model sharding across multiple GPUs via NVLink/PCIe is disabled. Execution enforces a strict `1 ModelInstance = 1 Physical Device` constraint. Models must fit entirely within a single node's discrete VRAM.
3. **Performance Floor (CPU Fallback):** If `magnetar-cuda` fails to initialize on a node, Tachyon will gracefully fall back to Magnetar's `Reference CPU Provider`, prioritizing execution success over latency.