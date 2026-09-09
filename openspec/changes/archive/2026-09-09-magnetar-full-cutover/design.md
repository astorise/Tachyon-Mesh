# Design: Magnetar Integration Architecture

## 1. Dependency Purge
All crates prefixed with `candle-` (`candle-core`, `candle-nn`, `candle-transformers`, `candle-flash-attn`) must be aggressively purged from the Tachyon workspace. The sole inference dependency becomes `magnetar-runtime = "0.1.0"`.

## 2. Control Plane Refactoring (The Mesh Router)
Tachyon's node discovery and routing logic must stop attempting to probe hardware directly.
*   **Hardware Discovery:** Nodes will query Magnetar's `Provider API` to retrieve `Capability Advertisements`. Tachyon orchestrates workloads based strictly on these declared capabilities (e.g., `Device ID: CUDA_0, Memory: 24GB, Compute: FP16`).
*   **Session Orchestration:** Incoming gRPC/REST inference requests are no longer mapped to synchronous tensor operations. The router must map requests to Magnetar's `ExecutionStream` and push them into the `continuous_batching` event loop.

## 3. Host-WASM Boundary (Zero-Copy Enforcement)
Tachyon's host-side WASM bindings must be entirely rewritten to sever all direct memory access to tensors.
*   The WASM runtime within Tachyon must exclusively pass and receive opaque handles (`PreparedKernelId` and `TensorId`).
*   All RAM/VRAM allocation, pinning, and eviction policies are delegated entirely to Magnetar's Asynchronous Caching Allocator (Arena). Tachyon acts only as the I/O pipeline feeding the Arena.

## 4. I/O Delegation
Following the `magnetar-cli` pattern, Tachyon assumes full responsibility for I/O. Tachyon handles the network transport of model artifacts (e.g., fetching GGUF/Safetensors via the mesh) and passes raw memory-mapped pointers (via `mmap`) directly to Magnetar.