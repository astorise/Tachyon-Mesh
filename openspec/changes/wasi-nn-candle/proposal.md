# Proposal: Change 042 - Native AI Inference (WASI-NN & Candle Batching)

## Context
Standard WebAssembly instances are ephemeral, which is fundamentally incompatible with the memory requirements of Large Language Models (LLMs) that require weights to be "pinned" in GPU VRAM. Furthermore, the `wasi-nn` specification provides a standard interface for FaaS to request inference, but it does not handle hardware concurrency. If multiple FaaS instances invoke `wasi-nn` simultaneously without orchestration, the GPU will run Out of Memory (OOM). We need a native Rust backend to manage the GPU state and batch concurrent requests.

## Objective
1. Integrate the `candle` crate into the Tachyon `core-host` to act as the backend for the `wasi-nn` standard.
2. Implement a "Continuous Batching" scheduler in the host. When multiple FaaS instances call `compute()`, the host pauses their execution, groups their prompts into a single matrix (Tensor), and executes a unified forward pass on the GPU.
3. Pre-load models into VRAM based on the `integrity.lock` lifecycle settings to guarantee zero-cold-start inference.

## Scope
- Implement the `wasi:nn/tensor` and `wasi:nn/inference` WIT interfaces in the Rust host.
- Build a dedicated background thread in the host that holds the `candle::Device` (CUDA/Metal) and the model weights.
- Create an asynchronous MPSC (Multi-Producer, Single-Consumer) queue to bridge the FaaS requests with the GPU batching thread.

## Success Metrics
- A 2MB User FaaS compiled to `wasm32-wasi` successfully generates text using a 7B parameter model without embedding the model.
- 50 concurrent requests to the FaaS result in a single, batched GPU computation rather than 50 sequential or crashing computations, maximizing Token/s throughput.