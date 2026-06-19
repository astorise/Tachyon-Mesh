# Implementation Tasks

- [ ] **Task 1: ONNX CUDA op allow-list**
  - Build the validated-operator allow-list (`OnnxOpSupport`) by running the existing ONNX test fixtures on `Device::Cuda` and recording which `candle-onnx` ops produce correct results vs. issue #3491 failures.
  - Document which ops are blocked upstream so the list can be revisited when candle is upgraded.

- [ ] **Task 2: ONNX device routing**
  - Update `CandleOnnxBackend`/`CandleOnnxGraph` to construct tensors on the model binding's declared GPU device when every operator in the graph is in the allow-list; otherwise construct on CPU.
  - Add `executed_on` field to the inference response/telemetry path.

- [ ] **Task 3: NVFP4 native forward pass**
  - Implement `Nvfp4Linear::forward` dispatching to the compiled CUDA/CUTLASS dequant+matmul kernels when `nvfp4-cuda` is enabled and capability checks pass.
  - Wire it into the model forward graph for classified ModelOpt/NVFP4 component sets, replacing the unconditional unsupported-execution return.

- [ ] **Task 4: NVFP4 fallback execution path**
  - Implement the BF16/F32-fallback-then-GPU-matmul path for accelerators without native FP4 kernels, respecting configured memory limits.
  - Preserve the existing typed unsupported-execution error as the last-resort outcome when neither native nor fallback execution fits.

- [ ] **Task 5: Telemetry**
  - Emit `executed_on` (cpu / gpu-native-fp4 / gpu-fallback / gpu-onnx) per inference call into the existing `compute-observability` pipeline.

- [ ] **Task 6: Tests**
  - GPU-gated integration tests (skipped without CUDA hardware in CI, run via a labeled hardware lane) verifying ONNX allow-listed ops produce numerically equivalent output on CPU vs. GPU.
  - NVFP4 forward pass test: synthetic fixture produces equivalent output via native-FP4 path and via fallback path.
  - Regression test: non-allow-listed ONNX ops and accelerators without `nvfp4-cuda` still behave exactly as before this change (CPU / unsupported-execution error respectively).

- [ ] **Task 7: Docs**
  - Update `CHANGELOG.md` to replace the "GPU inference is deferred pending upstream candle issue #3491 (CPU-only for now)" line with the actual current state (allow-listed GPU ops + remaining CPU-only ops) once Task 2 lands.
  - Document the NVFP4 execution path selection in the `modelopt-nvfp4-kernels` operator runbook.
