# ai-inference Specification

## Purpose
TBD - created by archiving change ai-inference-wasinn. Update Purpose after archive.
## Requirements
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

### Requirement: Host configuration can bind named preloaded models for AI targets
The integrity manifest SHALL allow AI-capable targets to declare model aliases, storage paths, and
target devices so the host can preload model bindings before serving inference.

#### Scenario: A target declares a GPU-backed model binding
- **WHEN** a target configuration defines a model alias, model path, and device
- **THEN** the host loads that model binding into its runtime configuration for startup initialization

### Requirement: Inference requests are continuously batched by the host
The host SHALL run a batching scheduler that groups compatible inference requests within a short
time window and executes them as a single Candle-backed forward pass.

#### Scenario: Multiple inference requests arrive together
- **WHEN** several inference requests are queued within the batching window
- **THEN** the scheduler pads and batches them into a single model execution
- **AND** routes each generated response back to the correct caller

### Requirement: WASI-NN calls are bridged through the batching scheduler
The Wasmtime host SHALL intercept `wasi-nn` compute calls, enqueue them with response channels,
and resume the guest only after the scheduler returns inference output.

#### Scenario: A guest invokes `wasi-nn` compute against a preloaded alias
- **WHEN** a guest module issues a `wasi-nn` compute request for a preloaded model alias
- **THEN** the host packages the inputs into an inference request
- **AND** submits it to the batching scheduler
- **AND** writes the resulting output back into guest memory before resuming execution

### Requirement: CI validates the optional AI inference build path
The repository SHALL build the `guest-ai` artifact in CI and validate that the optional
`core-host --features ai-inference` path still compiles.

#### Scenario: GitHub Actions checks the optional AI feature
- **WHEN** the main CI workflow runs on GitHub Actions
- **THEN** it builds `guest-ai` for `wasm32-wasip1`
- **AND** it runs `cargo check -p core-host --features ai-inference`
- **AND** it still builds the default `core-host` release artifact without the feature

### Requirement: Wasm guests may request a LoRA adapter for an inference call
The Mesh SHALL extend the `wit/ai` Wasm Component Model definitions so that an inference call accepts an optional `adapter_id` parameter, allowing a guest to request that a tenant-specific LoRA adapter be applied to the shared foundation model for that single call.

#### Scenario: Guest requests an adapter that is locally available
- **WHEN** a Wasm guest invokes the inference interface with an `adapter_id`
- **AND** the corresponding `.safetensors` adapter exists in `system-faas-model-broker`
- **THEN** the host loads the adapter weights and applies them to the foundation model's execution graph
- **AND** the inference output reflects the adapter's behaviour
- **AND** guests that omit `adapter_id` continue to run against the unmodified foundation model

### Requirement: Candle engine hot-swaps adapter weights and bounds context-switching overhead
The `wasi-nn-candle` execution engine SHALL dynamically inject and remove `.safetensors` adapter matrices during inference and SHALL bound the rate of adapter context-switching so that the cost of switching between adapters cannot dominate end-to-end latency.

#### Scenario: Concurrent tenants alternate adapters without runaway switching
- **WHEN** multiple tenants issue back-to-back inference calls with different `adapter_id` values
- **THEN** the engine swaps adapter weights on the shared foundation model between calls
- **AND** the swap operation occurs without reloading the foundation model into VRAM
- **AND** the engine enforces the configured maximum adapter-switch rate to keep aggregate latency within target SLOs

### Requirement: Inference workloads MUST support declarative LoRA Multiplexing
The `system-faas-model-broker` SHALL allow the sharing of a single base model in VRAM across multiple tenants by dynamically loading LoRA (Low-Rank Adaptation) weights based on Layer 7 routing conditions defined in the GitOps configuration.

#### Scenario: Routing to a tenant-specific LoRA
- **GIVEN** a base model pinned in VRAM and a configured LoRA adapter for the "legal" domain
- **WHEN** an inference request arrives with the header `X-Tenant-Domain: legal`
- **THEN** the Candle engine hot-swaps the "legal" LoRA adapter into the computation graph
- **AND** processes the prompt without reloading the base model weights, achieving zero-overhead multi-tenancy.

### Requirement: Large Models MUST support declarative Tensor Parallelism
The orchestration configuration SHALL allow operators to define a `tensor_parallelism` strategy, forcing the underlying `wasi-nn` backend to partition model layers across multiple available GPUs to prevent OOM errors on large models.

#### Scenario: Partitioning a model across GPUs
- **GIVEN** an AI deployment configured with `tensor_parallelism`
- **WHEN** the model broker loads a model that exceeds a single GPU's available VRAM
- **THEN** the runtime partitions model layers across the configured GPU set
- **AND** rejects startup with a typed configuration error if the requested GPU topology is unavailable.

### Requirement: AI inference bindings MUST classify ModelOpt/NVFP4 directories without mock execution
The AI inference runtime SHALL load supported ModelOpt/NVFP4 model bindings as typed component sets and SHALL NOT return mock inference output for those aliases.

#### Scenario: Detected NVFP4 alias refuses execution until backend is configured
- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** a guest or host caller submits an inference request for that alias
- **THEN** the runtime returns an actionable unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Existing ONNX guest path remains available
- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** ModelOpt/NVFP4 loading does not change the ONNX graph encoding contract

### Requirement: Unsupported quantized model bindings MUST fail with actionable errors
The AI inference runtime SHALL reject unsupported quantized model bindings with typed errors before returning mock inference output.

#### Scenario: Unsupported ModelOpt layout is configured
- **WHEN** a model binding points at a ModelOpt/NVFP4 checkpoint whose tensor names, scale layout, or architecture are not supported
- **THEN** model initialization fails with a typed error containing the model alias, model path, and unsupported layout detail
- **AND** inference for that alias is not registered

#### Scenario: Non-NVFP4 model remains outside the NVFP4 loader
- **WHEN** a model binding points at a safetensors directory without NVFP4 metadata or NVFP4 scale tensors
- **THEN** the ModelOpt/NVFP4 loader does not claim the binding
- **AND** the host either routes the model to another supported backend or returns an unsupported-model error

### Requirement: Candle LLM bindings MUST generate real model output
The AI inference runtime SHALL execute supported local Candle text-generation model bindings by loading their tokenizer, config, and safetensors weights, and SHALL return generated UTF-8 text bytes instead of mock inference output.

#### Scenario: Supported Candle LLM binding returns generated text
- **WHEN** a model binding points at a supported local Candle LLM directory
- **AND** a guest or host caller submits a UTF-8 prompt as the first `U8` input tensor
- **THEN** the runtime loads the model tokenizer, config, and safetensors weights
- **AND** executes bounded text generation through Candle
- **AND** returns UTF-8 generated text bytes that are not `MOCK_LLM_RESPONSE`

#### Scenario: Supported Candle LLM binding accepts a bounded JSON request
- **WHEN** a model binding points at a supported local Candle LLM directory
- **AND** the first `U8` input tensor is a JSON generation request with `prompt` and optional generation parameters
- **THEN** the runtime validates the request against configured prompt and generation limits
- **AND** returns UTF-8 generated text bytes through the existing inference response path

### Requirement: Non-mock model bindings MUST NOT fall back to mock output
The AI inference runtime SHALL classify model bindings as explicit mock, ModelOpt/NVFP4, supported Candle LLM, ONNX/WASI-NN, or unsupported, and SHALL NOT return `MOCK_LLM_RESPONSE` for any non-mock binding.

#### Scenario: Unsupported safetensors directory fails before registration
- **WHEN** a model binding points at a safetensors directory that is neither ModelOpt/NVFP4 nor a supported Candle LLM
- **THEN** model initialization fails with a typed unsupported-model error containing the alias, path, and unsupported reason
- **AND** inference for that alias is not registered

#### Scenario: Runtime load failure does not use mock output
- **WHEN** a supported Candle LLM binding has invalid tokenizer, config, or weight files
- **THEN** model initialization fails with a typed load error containing the alias, path, and invalid component
- **AND** the runtime does not register a mock model for the alias

#### Scenario: Explicit mock binding preserves test behavior
- **WHEN** a test or fixture configures an explicit mock model binding
- **THEN** the runtime may return `MOCK_LLM_RESPONSE`
- **AND** the mock path remains distinguishable from supported Candle LLM bindings

### Requirement: Candle LLM generation MUST be bounded and deterministic by default
The Candle LLM runtime SHALL enforce prompt length, max-new-token, batch size, and sampling limits, and SHALL use deterministic generation defaults suitable for repeatable tests.

#### Scenario: Prompt exceeds configured limit
- **WHEN** a caller submits a prompt that exceeds the configured prompt token or byte limit
- **THEN** the runtime rejects the request with a typed validation error
- **AND** no generation work is executed

#### Scenario: Generation request omits sampling parameters
- **WHEN** a caller submits a plain UTF-8 prompt or a JSON request without sampling parameters
- **THEN** the runtime uses deterministic defaults for token selection
- **AND** repeated runs against the deterministic fixture produce the expected non-mock output

#### Scenario: Requested generation limit exceeds host cap
- **WHEN** a JSON generation request asks for more new tokens than the configured host cap
- **THEN** the runtime rejects or clamps the request according to the configured policy
- **AND** the behavior is reported in the response or error path

### Requirement: Existing ONNX and NVFP4 boundaries MUST remain unchanged
Adding a real Candle LLM runtime SHALL NOT change legacy Candle ONNX/WASI-NN graph loading or the ModelOpt/NVFP4 unsupported-execution boundary.

#### Scenario: Legacy ONNX guest still uses candle-onnx
- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** Candle LLM binding classification does not change the ONNX graph encoding contract

#### Scenario: ModelOpt/NVFP4 alias remains non-mock and unsupported for text generation
- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** no complete architecture execution runtime is configured for that alias
- **THEN** inference returns the existing actionable unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`

### Requirement: Real Candle LLM validation MUST run without network downloads
The repository SHALL include deterministic tests for real Candle LLM loading and generation that do not download external model artifacts during CI.

#### Scenario: CI validates real Candle generation
- **WHEN** the CI workflow runs the optional `core-host --features ai-inference` checks
- **THEN** it executes a deterministic real Candle LLM fixture test
- **AND** the fixture output is generated by Candle rather than by a mock backend
- **AND** the test does not require network access or Hugging Face downloads

#### Scenario: Optional real checkpoint probe is gated
- **WHEN** a developer sets an environment variable pointing at a local supported checkpoint directory
- **THEN** the test suite may run an additional real-checkpoint load and generation probe
- **AND** CI remains independent of that local checkpoint
