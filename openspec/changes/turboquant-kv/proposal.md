# Proposal: Change 043 - TurboQuant KV Cache (CUDA FFI & TDD)

## Context
Long-context LLM inference is severely bottlenecked by the memory footprint of the KV Cache (the attention state). Google's recent TurboQuant algorithm compresses this cache from `f16` (16 bits) down to 3 bits + a 1-bit residual (PolarQuant + QJL), effectively reducing VRAM usage by ~6x. Integrating this into Tachyon's native Candle backend requires custom CUDA C++ kernels bridged to Rust. Because FFI boundaries and GPU memory manipulation are highly prone to silent memory corruption and subtle math errors, this integration mandates a strict Test-Driven Development (TDD) methodology.

## Objective
1. Define the mathematical baseline of the TurboQuant compression in pure Rust (CPU fallback or mocked expected tensors) as the "Source of Truth".
2. Write unit tests that assert the equality between the baseline and the FFI CUDA output *before* implementing the C++ kernel.
3. Implement the custom CUDA C++ kernels and bind them to Rust using the `cc` crate.
4. Wrap the FFI in a `candle_core::CustomOp` to seamlessly inject it into the continuous batching scheduler (Change 042).

## Scope
- Setup the `build.rs` pipeline for `.cu` files.
- Create a comprehensive TDD test suite validating memory alignment, output shapes, and mathematical quantization thresholds.
- Implement the `TurboQuantCompressor` and `TurboQuantDecompressor` CustomOps in Candle.

## Success Metrics
- 100% of the Rust unit tests pass (`cargo test --features cuda`).
- The TDD test suite successfully catches deliberate off-by-one errors injected into the C++ kernel.
- In an end-to-end integration test, the KV Cache memory allocation drops by >80% with an accuracy degradation (Perplexity) of less than 0.1% compared to unquantized `f16`.