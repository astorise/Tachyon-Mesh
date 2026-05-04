# ai-inference Delta

## ADDED Requirements

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
