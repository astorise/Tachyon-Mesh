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

