## ADDED Requirements

### Requirement: WASM FaaS can submit a low-priority LoRA fine-tuning job
The Mesh SHALL expose a `wit/ai/training.wit` interface that allows a Wasm guest to submit a LoRA fine-tuning job to a local low-priority queue served by `system-faas-buffer`, without blocking the host or the inference critical path.

#### Scenario: Guest submits a LoRA training job
- **WHEN** a Wasm guest calls the `submit_training_job` interface with a model handle and a dataset reference
- **THEN** the host enqueues the job in the low-priority lane of `system-faas-buffer`
- **AND** the call returns a job identifier immediately to the guest
- **AND** the inference critical path continues to operate at unchanged latency

### Requirement: LoRA training tolerates limited VRAM via system RAM spillover
The Candle execution engine SHALL fall back to system RAM (CPU/RAM spillover) when accelerator VRAM is exhausted during backpropagation, and SHALL persist the resulting `.safetensors` adapter into `system-faas-model-broker` upon successful completion.

#### Scenario: Training completes on a VRAM-constrained Edge node
- **WHEN** a queued LoRA training job runs on a node with insufficient VRAM
- **THEN** the engine offloads tensors to system RAM rather than crashing with OOM
- **AND** training proceeds at degraded throughput but completes successfully
- **AND** the resulting `.safetensors` adapter is stored in `system-faas-model-broker`
- **AND** the job status is reported as `completed` to the originating tenant
