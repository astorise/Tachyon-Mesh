# Specifications: TDD & TurboQuant FFI Architecture

## 1. The TDD Strategy (Red-Green-Refactor for GPU)
Before writing the CUDA kernel, the test harness must be defined.
- **Red Phase:** Write a Rust test that generates a known `f16` input tensor. Calculate the expected 3-bit `i8` output and 1-bit `u8` residual mathematically in Rust. Call the (empty/stubbed) FFI function. The test fails.
- **Green Phase:** Implement the C++ CUDA kernel to perform the exact PolarQuant rotation and QJL quantization. The test passes when the GPU output matches the Rust expectation (within a defined floating-point epsilon/tolerance).
- **Refactor Phase:** Optimize the CUDA thread blocks, memory coalescing, and warp-level primitives, re-running the tests to ensure mathematical stability.

## 2. The FFI Boundary Contract
The Rust-to-C++ boundary must be unsafe but strictly typed:
- Input: `*const f16` (Device pointer to the original KV Cache).
- Outputs: `*mut i8` (Device pointer for the 3-bit PQ data), `*mut u8` (Device pointer for the 1-bit QJL data).
- Metadata: `seq_len: i32`, `head_dim: i32`, `stream: CUstream`.

## 3. Candle CustomOp Integration
The `TurboQuantCompressor` implements `CustomOp1`.
- `cuda_fwd` allocates the `i8` and `u8` zero-tensors on the GPU using Candle's `alloc_zeros`.
- It extracts the raw device pointers and invokes the `unsafe` FFI call.
- It returns the wrapped `CudaStorage` back to the model graph.