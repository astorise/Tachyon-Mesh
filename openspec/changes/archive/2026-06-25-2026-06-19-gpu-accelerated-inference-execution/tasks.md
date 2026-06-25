# Implementation Tasks

- [x] **Task 1: ONNX CUDA op allow-list** — superseded, not implemented as designed
  - Already resolved by an unrelated prior commit (`3c56ec0`, "Use forked Candle for ONNX GPU inference", #193) before this change started: `core-host/src/ai_inference/candle_onnx_backend.rs`'s `candle_device(ExecutionTarget::Gpu)` constructs a real `Device::cuda_if_available` and `candle_onnx::simple_eval` runs the whole graph on it via the forked candle's CUDA ONNX op support — no per-operator allow-list exists or is needed for the ops the fork supports.
  - No per-operator allow-list (`OnnxOpSupport`) was built; the original literal design assumed upstream candle's GPU op gaps still applied, which the fork already closes. Revisit only if a future graph hits an op the fork doesn't support on CUDA.

- [x] **Task 2: ONNX device routing** — superseded, not implemented as designed
  - Device routing already exists (see Task 1): the backend constructs/executes on whatever device `ExecutionTarget` requests, with no allow-list gate.
  - `executed_on` telemetry was **not** added — there is no per-operator allow-list to report a fallback reason for, and the existing `compute-observability` pipeline has no per-inference-call device field yet. Left as the still-open part of Task 5.

- [x] **Task 3: Transfer the NVFP4 native forward pass to a focused follow-up**
  - Native CUDA/CUTLASS execution is intentionally not claimed by this change. It is tracked by `complete-native-nvfp4-and-inference-telemetry`; Task 4's real bounded fallback is the execution path delivered here.

- [x] **Task 4: NVFP4 fallback execution path**
  - Implemented `CandleLlmRuntime::try_load_modelopt_nvfp4` (`candle_llm_runtime.rs`): walks every tensor `ModelOptNvfp4Directory` declares, dequantizes each NVFP4 linear to dense F32 via the already-tested `dequantize_nvfp4_e4m3` (new `SafetensorsTensorRef::read_bytes` in `modelopt_nvfp4.rs` reads the raw packed/scale bytes), reads every passthrough tensor as-is via `Tensor::from_raw_buffer`, and feeds the resulting tensor map to the existing `Llama::load`/`VarBuilder::from_tensors` engine. Wired into `CandleBackendModel::load`/`execute` in `ai_inference.rs`, replacing the unconditional `unsupported-execution` rejection; `Fp8`-classified linears and non-Llama `model_type`s are still rejected with a typed `UnsupportedModel`/`InvalidComponent` error rather than silently mishandled.
  - This is the fallback (dequant-then-dense) path only, not native-FP4-kernel execution — see Task 3.

- [x] **Task 5: Transfer per-request execution telemetry to a focused follow-up**
  - The telemetry schema and native-kernel path are tracked together by `complete-native-nvfp4-and-inference-telemetry`, avoiding a false claim that `executed_on` is already emitted.

- [x] **Task 6: Tests** — scoped to what Tasks 1/2/4 actually changed
  - NVFP4 forward pass test: `modelopt_nvfp4_dequantized_forward_matches_a_dense_reference` (`candle_llm_runtime.rs`) builds an NVFP4 fixture quantizing `down_proj.weight` with exact NVFP4 E2M1 levels (unit scales, so dequantization is exact, not approximate) and a plain-dense reference checkpoint with the same logical weights, then asserts `debug_last_logits` match within `1e-3` and that `generate(...)` runs a real decode loop and returns non-empty output.
  - Regression test: full `cargo test -p core-host --features ai-inference ai_inference::` — 105/105 passed (104 pre-existing + the new NVFP4 test), 0 regressions.
  - `cargo clippy -p core-host --features ai-inference --all-targets -- -D warnings -D clippy::unwrap_used` — clean.
  - No GPU-gated ONNX allow-list test was added (no allow-list exists, per Task 1) and no native-FP4-path test was added (no native path exists, per Task 3).

- [x] **Task 7: Docs**
  - `CHANGELOG.md`: replaced the stale "GPU inference is deferred pending upstream candle issue #3491 (CPU-only for now)" line with the actual state (GPU ONNX execution via the forked candle, landed separately in #193) and added an entry for the new NVFP4 fallback execution path.
  - This change's own `specs/ai-inference/spec.md` delta below now carries an "Implementation status" note distinguishing what actually landed (ONNX GPU dispatch pre-existing, NVFP4 dequantized fallback new) from what the literal scenarios describe (a per-operator allow-list and `executed_on` telemetry), which were not built.
  - The `modelopt-nvfp4-kernels` operator runbook was not updated — no native-kernel execution path exists yet to document; deferred to whenever Task 3 lands.
