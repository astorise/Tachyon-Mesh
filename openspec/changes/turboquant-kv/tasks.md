# Tasks: Change 043 Implementation

**Agent Instruction:** Follow Test-Driven Development. Do not write the CUDA kernel until the Rust tests and FFI signatures are defined.

- [ ] Define the TurboQuant Rust module, FFI signatures, and failing CUDA-backed test harness before writing the kernel.
- [ ] Extend the build pipeline to compile `src/cuda/turboquant_kernels.cu` through `cc` with the required CUDA architecture flags.
- [ ] Implement the TurboQuant CUDA kernel and launcher until the Rust tests pass.
- [ ] Wire the CUDA launcher into a Candle `CustomOp1` implementation and validate the compression ratio on an integration workload.

## Validation Notes
1. Run `cargo test --features cuda` after the test harness and kernel are in place.
2. Measure GPU memory before and after the `TurboQuantCompressor` on a representative KV-cache workload and verify the expected reduction.
