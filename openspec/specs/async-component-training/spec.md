# async-edge-train-training Specification

## Purpose
TBD - created by archiving change async-edge-train-training. Update Purpose after archive.
## Requirements
### Requirement: WASM FaaS can submit a low-priority Component training fine-tuning job
The Mesh SHALL expose a `wit/ai/training.wit` interface that allows a Wasm guest to submit a Component training fine-tuning job to a local low-priority queue served by `system-faas-buffer`, without blocking the host or the inference critical path.

#### Scenario: Guest submits a Component training job
- **WHEN** a Wasm guest calls the `submit_training_job` interface with a model handle and a dataset reference
- **THEN** the host enqueues the job in the low-priority lane of `system-faas-buffer`
- **AND** the call returns a job identifier immediately to the guest
- **AND** the inference critical path continues to operate at unchanged latency

### Requirement: Component training tolerates limited VRAM via system RAM spillover
The training execution engine SHALL fall back to system RAM (CPU/RAM spillover) when accelerator VRAM is exhausted during backpropagation, and SHALL persist the resulting `.training.json` adapter into `system-faas-model-broker` upon successful completion.

#### Scenario: Training completes on a VRAM-constrained Edge node
- **WHEN** a queued Component training job runs on a node with insufficient VRAM
- **THEN** the engine offloads tensors to system RAM rather than crashing with OOM
- **AND** training proceeds at degraded throughput but completes successfully
- **AND** the resulting `.training.json` adapter is stored in `system-faas-model-broker`
- **AND** the job status is reported as `completed` to the originating tenant

### Requirement: Operators can query Component training job status
The host SHALL expose an authenticated admin status endpoint for Component training jobs, and Tachyon MCP SHALL expose a `tachyon_component_training_status` tool that returns the current queue state for a submitted training job id.

#### Scenario: MCP reads a Component training job status
- **GIVEN** a Component training job has been submitted through the training WIT interface
- **WHEN** an operator or AI agent calls `tachyon_component_training_status` with that `job_id`
- **THEN** the MCP server queries the node's admin Component training status endpoint
- **AND** returns `queued`, `running`, `completed`, or `failed` with progress or artifact details when available

#### Scenario: Unknown Component training job is reported clearly
- **WHEN** the admin Component training status endpoint is queried with an unknown `job_id`
- **THEN** the host returns a not-found response
- **AND** the MCP client surfaces the missing job without inventing a queue state
