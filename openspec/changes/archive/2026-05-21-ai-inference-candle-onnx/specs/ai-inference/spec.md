## MODIFIED Requirements

### Requirement: Host optionally exposes WASI-NN imports to legacy guests
The `core-host` runtime SHALL define an `ai-inference` Cargo feature that links the `wasi_ephemeral_nn` preview1 host functions for legacy WASI guests without changing the default host build. The feature SHALL use `candle-onnx` (pure Rust) as the ONNX inference backend, making `--features ai-inference` compatible with musl libc targets.

#### Scenario: Default host builds without AI inference
- **WHEN** a developer builds `core-host` without enabling `ai-inference`
- **THEN** the host compiles successfully without `wasmtime-wasi-nn` or `candle-onnx`
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links WASI-NN via candle-onnx backend
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the legacy preview1 linker registers the `wasi_ephemeral_nn` imports
- **AND** legacy guests can resolve the `wasi-nn` host functions at instantiation time
- **AND** the build succeeds on musl libc targets (Alpine) without native library dependencies

#### Scenario: ONNX model loaded from raw bytes via CandleOnnxBackend
- **WHEN** a legacy guest calls `graph_load` with raw ONNX model bytes and encoding `onnx`
- **THEN** the host decodes the bytes into a `ModelProto` via `prost`
- **AND** constructs a `CandleOnnxGraph` backed by candle-onnx's `simple_eval`
- **AND** returns a graph handle to the guest without touching the filesystem

### Requirement: AI guest reads sealed ONNX models and returns JSON inference output
The workspace SHALL include a `guest-ai` legacy guest that reads a JSON tensor request, loads an ONNX model from a sealed read-only `/models` directory, runs inference via `wasi-nn` using the candle-onnx backend, and returns the output tensor as JSON. Inference executes on CPU; GPU execution is deferred pending upstream candle fix (issue #3491).

#### Scenario: Valid request loads a sealed model and computes inference
- **WHEN** `/api/guest-ai` is sealed with a read-only volume mounted at `/models`
- **AND** the client sends a JSON request containing `shape`, `values`, and `output_len`
- **THEN** `guest-ai` loads the requested ONNX model from `/models`
- **AND** it calls `set_input`, `compute`, and `get_output` via WASI-NN witx
- **AND** the candle-onnx backend executes the model on CPU
- **AND** it returns a JSON response containing the output tensor values

#### Scenario: Invalid request body returns a JSON error payload
- **WHEN** the client sends malformed JSON or tensor dimensions that do not match the input values
- **THEN** `guest-ai` does not attempt inference
- **AND** it returns a JSON payload describing the validation error
