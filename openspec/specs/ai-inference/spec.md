# ai-inference Specification

## Purpose
TBD - created by archiving change ai-inference-wasinn. Update Purpose after archive.
## Requirements
### Requirement: Host optionally exposes WASI-NN imports to legacy guests
The `core-host` runtime SHALL define an `ai-inference` Cargo feature that links the
`wasi_ephemeral_nn` preview1 host functions for legacy WASI guests without changing the default
host build.

#### Scenario: Default host builds without AI inference
- **WHEN** a developer builds `core-host` without enabling `ai-inference`
- **THEN** the host compiles successfully without `wasmtime-wasi-nn`
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links WASI-NN
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the legacy preview1 linker registers the `wasi_ephemeral_nn` imports
- **AND** legacy guests can resolve the `wasi-nn` host functions at instantiation time

### Requirement: AI guest reads sealed ONNX models and returns JSON inference output
The workspace SHALL include a `guest-ai` legacy guest that reads a JSON tensor request, loads an
ONNX model from a sealed read-only `/models` directory, runs inference via `wasi-nn`, and returns
the output tensor as JSON.

#### Scenario: Valid request loads a sealed model and computes inference
- **WHEN** `/api/guest-ai` is sealed with a read-only volume mounted at `/models`
- **AND** the client sends a JSON request containing `shape`, `values`, and `output_len`
- **THEN** `guest-ai` loads the requested ONNX model from `/models`
- **AND** it calls `set_input`, `compute`, and `get_output`
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

