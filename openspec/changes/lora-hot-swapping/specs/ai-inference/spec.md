## ADDED Requirements

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
