# ai-inference Delta

## MODIFIED Requirements

### Requirement: AI guest reads sealed ONNX models and returns JSON inference output
The workspace SHALL include a `guest-ai` legacy guest that reads a JSON tensor request, loads an ONNX model from a sealed read-only `/models` directory, runs inference via `wasi-nn` using the candle-onnx backend, and returns the output tensor as JSON. Inference SHALL execute on the GPU device declared by the model binding when every operator in the loaded graph is validated for GPU execution; otherwise it executes on CPU.

#### Scenario: Valid request loads a sealed model and computes inference
- **WHEN** `/api/guest-ai` is sealed with a read-only volume mounted at `/models`
- **AND** the client sends a JSON request containing `shape`, `values`, and `output_len`
- **THEN** `guest-ai` loads the requested ONNX model from `/models`
- **AND** it calls `set_input`, `compute`, and `get_output` via WASI-NN witx
- **AND** the candle-onnx backend executes the model on the model binding's declared device when all graph operators are GPU-validated, or on CPU otherwise
- **AND** it returns a JSON response containing the output tensor values

#### Scenario: Invalid request body returns a JSON error payload
- **WHEN** the client sends malformed JSON or tensor dimensions that do not match the input values
- **THEN** `guest-ai` does not attempt inference
- **AND** it returns a JSON payload describing the validation error

#### Scenario: GPU-validated ONNX graph executes on the declared GPU device
- **GIVEN** a model binding declares a GPU device
- **AND** every operator in the loaded ONNX graph is in the validated CUDA-safe operator allow-list
- **WHEN** `guest-ai` runs inference for that model
- **THEN** the candle-onnx backend constructs and executes tensors on that GPU device
- **AND** the inference response/telemetry records `executed_on: gpu`

#### Scenario: Non-allow-listed operator falls back to CPU explicitly
- **GIVEN** a model binding declares a GPU device
- **AND** the loaded ONNX graph contains at least one operator not in the validated CUDA-safe allow-list
- **WHEN** `guest-ai` runs inference for that model
- **THEN** the candle-onnx backend executes on CPU
- **AND** the inference response/telemetry records `executed_on: cpu` with reason `unsupported_op_on_gpu`
- **AND** the result is correct (not silently degraded) even though it did not use the declared GPU

## ADDED Requirements

### Requirement: Inference execution device MUST be observable per request
The runtime SHALL record, for every inference call, which device class actually executed it (`cpu`, `gpu`, `gpu-native-fp4`, or `gpu-fallback`) in the existing compute-observability telemetry pipeline.

#### Scenario: Operator can distinguish GPU execution from silent CPU fallback
- **WHEN** an operator inspects telemetry for an inference call against a model binding that declares a GPU device
- **THEN** the telemetry indicates the actual device class that executed the call
- **AND** a CPU fallback is distinguishable from genuine GPU execution without reading source code

## Implementation status as of this change

ONNX GPU execution (the "MODIFIED" requirement above) was already real before this
change started: an unrelated prior commit (`3c56ec0`, #193) wired
`candle_onnx_backend.rs`'s `ExecutionTarget::Gpu` to a real CUDA `Device` and
`candle_onnx::simple_eval` via the forked candle's CUDA ONNX op support. The
per-operator allow-list (`OnnxOpSupport`) and `executed_on: gpu`/`executed_on:
cpu` telemetry the scenarios above describe were **not** built — the fork's
CUDA ONNX coverage made a manual allow-list unnecessary for the ops exercised
so far, and there is no per-inference-call device telemetry field anywhere in
`compute-observability` yet to populate. The "ADDED Requirement" (observable
`executed_on` per call) is therefore still unimplemented and tracked as a
follow-up rather than delivered here.

What this change actually delivers is the NVFP4 (ModelOpt) side: ModelOpt/
NVFP4 Llama checkpoints, previously detected and load-time-validated but
unconditionally rejected at execution time, now run via a real dequantize-to-
dense-F32-then-execute fallback (`CandleLlmRuntime::try_load_modelopt_nvfp4`
in `candle_llm_runtime.rs`, dispatched from `CandleBackendModel::load`/
`execute` in `ai_inference.rs`). Native FP4-kernel matmul without eager
dequantization (eliminating the dequantization memory/time overhead) remains
out of scope and is tracked as a follow-up, as is combining NVFP4 with
tensor/pipeline/expert parallelism in the same deployment. Verified by a new
load-and-forward equivalence test
(`modelopt_nvfp4_dequantized_forward_matches_a_dense_reference`) and the full
`ai_inference::` suite (105 tests, 0 regressions).
