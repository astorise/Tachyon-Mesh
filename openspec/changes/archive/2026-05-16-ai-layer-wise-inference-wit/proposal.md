# Proposal: Layer-Wise AI Inference via Tachyon AI WIT Contract

## Context
Standard AI execution in WebAssembly often defaults to `wasi-nn`, which forces a monolithic input-to-output black-box approach. This abstraction fails on resource-constrained Tier 3 edge nodes (e.g., home servers or decentralized infrastructure) that need to run large local models layer-by-layer due to tight VRAM limitations. Passing raw multi-gigabyte tensors back and forth across the Wasm linear memory boundary satisfies the isolated design but introduces severe PCIe bus saturation and memory fragmentation.

## Problem
1. **Abstraction Friction:** `wasi-nn` does not allow the Wasm guest to control hardware cache shifting or target fine-grained layers inside a `.safetensors` memory map (`mmap`).
2. **Boundary Overhead:** Copying intermediate hidden states (tensors) inside and out of the Wasm module breaks performance promises.
3. **Core Bloat:** Compiling comprehensive linear algebra, matrix multiplication kernels, and large dependencies like `candle-core` or CUDA/Metal interfaces by default increases the binary size and execution foot-print of the `core-host` for standard light-weight deployments.

## Proposed Solution
Introduce a dedicated, zero-copy WIT interface in the existing `tachyon:mesh` AI package built around Major Component Model features. The Wasm module acts solely as an orchestrator/sequencer, manipulating opaque 32-bit resource integers (`tensor-handle`) that point to tensors stored safely in native host VRAM. 

To guarantee an absolute minimum runtime footprint, all heavy weight matrix dependencies are isolated behind a conditional compilation Rust feature (`ai-inference`). When compiled out, the host retains its ultra-lean footprint.

## Points of Attention & Mitigations
- **VRAM Garbage Collection:** Opaque `u32` resource IDs risk memory leaks if a FaaS component finishes execution, times out, or panics without releasing its tensor handles. 
  - *Mitigation:* Explicitly tie the host tensor lifetime mapping to the specific Wasmtime `Store` context. When the instance context is destroyed, an immediate drop hook frees the associated host VRAM allocations.
- **Lock Contention during Micro-Batching:** Multi-threaded FaaS workers executing horizontal batch passes on `forward-layer` can run into read/write lock bottlenecks.
  - *Mitigation:* Ensure safe model configuration layers are mapped read-only using shared references (`Arc<Tensor>`) and utilize non-blocking token tracking or lock-free structural maps (`dashmap`) for lookups.
