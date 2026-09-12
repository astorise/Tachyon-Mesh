# memory-delegation Specification

## Purpose
Specify the post-cutover memory boundary between Tachyon and Magnetar. Tachyon may transport and authorize model artifacts, but Magnetar owns model tensor allocation, KV-cache management, Provider memory, and admission decisions exposed by its public runtime contracts.
## Requirements
### Requirement: Magnetar MUST own runtime model memory
Tachyon-Mesh SHALL delegate model tensor allocation, pinning, eviction, KV-cache management, and VRAM/RAM ownership decisions to Magnetar's runtime and Providers. Tachyon SHALL NOT maintain a pseudo Magnetar arena or Tachyon-owned tensor resource manager for production Qwen execution.

#### Scenario: Loading a Qwen artifact bundle
- **GIVEN** Tachyon has staged an authorized local bundle for a supported Qwen model
- **WHEN** the model loader prepares the artifact for inference
- **THEN** Tachyon SHALL pass the authorized bundle to Magnetar production ingestion
- **AND** Magnetar SHALL own manifest normalization, payload access, tensor materialization, and Provider memory
- **AND** Tachyon host code SHALL NOT parse Safetensors payloads, convert model tensor dtypes, construct Qwen graphs, or manage runtime tensor buffers

### Requirement: Admission Capacity Check
Before accepting or routing a local inference session based on model memory capacity, Tachyon-Mesh SHALL use Magnetar Provider/runtime admission signals when they are exposed. Tachyon SHALL NOT substitute fixed KV-cache byte counters or hardcoded VRAM estimates as proof of capacity.

#### Scenario: Magnetar reports sufficient capacity
- **GIVEN** a node advertises a Magnetar provider capable of serving Qwen inference
- **AND** Magnetar reports enough capacity for the requested generation
- **WHEN** an inference request is admitted locally
- **THEN** Tachyon SHALL enqueue the request for Magnetar execution

#### Scenario: Magnetar reports OOM
- **GIVEN** a node advertises a Magnetar provider capable of serving Qwen inference
- **AND** Magnetar reports insufficient capacity for the requested generation
- **WHEN** an inference request targets that node
- **THEN** Tachyon SHALL reject the local request with a resource-exhaustion status
- **AND** the mesh router SHALL try another node with matching Qwen capabilities when one is available

#### Scenario: Magnetar does not expose a memory signal
- **GIVEN** Magnetar does not expose a Provider/runtime memory admission signal for a requested placement
- **WHEN** Tachyon publishes mesh capabilities for that placement
- **THEN** Tachyon SHALL omit unavailable memory capacity telemetry or mark it unknown
- **AND** Tachyon SHALL NOT publish hardcoded memory values as if they came from Magnetar
