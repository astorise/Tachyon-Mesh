# Tasks: TurboQuant Integration

## Phase 1: Ground Truth Extraction (C++)
- [x] **Task 1.1:** Create a temporary C++ script (or modify `test-quantize-fns.cpp` in the submodule) to generate test fixtures. It must initialize a random `float32` tensor, compress it using the TurboQuant V-compression function, and dump BOTH the original tensor and the compressed binary array into `fixtures/v_tensor_f32.bin` and `fixtures/v_tensor_tq.bin`.

## Phase 2: FFI Bridge & Rust Project Setup
- [x] **Task 2.1:** Create a new Rust crate (e.g., `turboquant-sys`).
- [x] **Task 2.2:** Write a `build.rs` script using the `cc` crate to compile the necessary CUDA/C++ files from the TurboQuant submodule. Enable CUDA/Metal flags conditionally.
- [x] **Task 2.3:** Write the Rust `extern "C"` block mapping the compression and decompression (Sparse V) functions.

## Phase 3: Candle CustomOp Implementation
- [x] **Task 3.1:** In the main Rust worker, implement the `candle_core::CustomOp1` (or `CustomOp2`) trait for a new struct `TurboQuantDecompressor`.
- [x] **Task 3.2:** Write the `forward` method of the `CustomOp` to safely pass the Candle tensor pointers (`.as_ptr()`) to the FFI C functions. Ensure shapes and strides are validated.

## Phase 4: Unit Testing (TDD)
- [x] **Task 4.1:** Write a Rust unit test `test_turboquant_ffi_match`. It must read the binary fixtures generated in Task 1.1, run the Candle `CustomOp`, and assert that the output matches the reference binary data bit-for-bit. **(Do not proceed to Phase 5 until this test passes).**

## Phase 5: Model Integration
- [x] **Task 5.1:** Locate the Attention mechanism in the chosen Candle model file (e.g., Llama/Mistral).
- [x] **Task 5.2:** Implement the Boundary Layer Protection logic: skip TurboQuant for the first 2 and last 2 layers.
- [x] **Task 5.3:** Implement the Asymmetric Cache logic: ensure only $V$ is passed to `TurboQuantDecompressor`, while $K$ remains `q8_0`.
- [x] **Task 5.4:** Hook the attention weights (post-softmax) into the `TurboQuantDecompressor` to enable Sparse V decoding.

## Phase 6: Validation
- [x] **Task 6.1:** Run a full integration test with a dummy Pacbase-like prompt. Verify that VRAM usage is strictly bounded and that the output logits do not degrade compared to a pure `f16` inference run.
