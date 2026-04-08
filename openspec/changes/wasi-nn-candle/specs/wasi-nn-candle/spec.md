## ADDED Requirements

### Requirement: Host configuration can bind named models for AI targets
The integrity manifest SHALL allow AI-capable targets to declare model aliases, storage paths, and target devices so the host can preload model weights before serving inference.

#### Scenario: A target declares a GPU-backed model binding
- **WHEN** a target configuration defines a model alias, model path, and device
- **THEN** the host loads that model binding into its runtime configuration for startup initialization

### Requirement: Inference requests are continuously batched by the host
The host SHALL run a batching scheduler that groups compatible inference requests within a short time window and executes them as a single Candle forward pass.

#### Scenario: Multiple inference requests arrive together
- **WHEN** several inference requests are queued within the batching window
- **THEN** the scheduler pads and batches them into a single model execution
- **AND** routes each generated response back to the correct caller

### Requirement: WASI-NN calls are bridged through the batching scheduler
The Wasmtime host SHALL intercept `wasi-nn` compute calls, enqueue them with response channels, and resume the guest only after the scheduler returns inference output.

#### Scenario: A guest invokes `wasi-nn` compute
- **WHEN** a guest module issues a `wasi-nn` compute request
- **THEN** the host packages the inputs into an inference request
- **AND** submits it to the batching scheduler
- **AND** writes the resulting output back into guest memory before resuming execution
