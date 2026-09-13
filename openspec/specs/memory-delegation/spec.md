# memory-delegation Specification

## Purpose
Specify the post-cutover memory boundary between Tachyon and Magnetar. Tachyon
may transport and authorize inference Component artifacts, but Magnetar owns
model tensor allocation, KV-cache management, Provider memory, and admission
decisions exposed by its public runtime contracts.

## Requirements
### Requirement: Magnetar MUST own runtime inference memory
Tachyon-Mesh SHALL delegate model tensor allocation, pinning, eviction, KV-cache
management, and VRAM/RAM ownership decisions to Magnetar's runtime and
Providers. Tachyon SHALL NOT maintain a pseudo Magnetar arena, Tachyon-owned
tensor resource manager, or model-family-specific residency handle for local
Component execution.

#### Scenario: Loading an inference Component artifact
- **GIVEN** Tachyon has staged an authorized local inference Component artifact
- **WHEN** the Component is prepared for inference
- **THEN** Tachyon SHALL pass the authorized source and placement constraints to
  Magnetar
- **AND** Magnetar SHALL own manifest normalization, payload access, tensor
  materialization, model instance lifecycle, and Provider memory
- **AND** Tachyon host code SHALL NOT parse model payloads, convert model tensor
  dtypes, construct model graphs, or manage runtime tensor buffers

### Requirement: Admission Capacity Check
Tachyon-Mesh SHALL use Magnetar Provider/runtime admission signals when they are
exposed before accepting or routing a local inference session based on memory
capacity. Tachyon SHALL NOT substitute fixed KV-cache byte counters or
hardcoded VRAM estimates as proof of capacity.

#### Scenario: Magnetar reports sufficient capacity
- **GIVEN** a node advertises a Magnetar Component placement capable of serving
  local inference
- **AND** Magnetar reports enough capacity for the requested invocation
- **WHEN** an inference request is admitted locally
- **THEN** Tachyon SHALL enqueue the request for Magnetar execution

#### Scenario: Magnetar reports OOM
- **GIVEN** a node advertises a Magnetar Component placement capable of serving
  local inference
- **AND** Magnetar reports insufficient capacity for the requested invocation
- **WHEN** an inference request targets that node
- **THEN** Tachyon SHALL reject the local request with a resource-exhaustion
  status
- **AND** the mesh router SHALL try another node with matching generic
  placement capabilities when one is available

#### Scenario: Magnetar does not expose a memory signal
- **GIVEN** Magnetar does not expose a Provider/runtime memory admission signal
  for a requested placement
- **WHEN** Tachyon publishes mesh capabilities for that placement
- **THEN** Tachyon SHALL omit unavailable memory capacity telemetry or mark it
  unknown
- **AND** Tachyon SHALL NOT publish hardcoded memory values as if they came from
  Magnetar
