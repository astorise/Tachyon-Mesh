# memory-delegation Specification

## Purpose
TBD - created by archiving change magnetar-full-cutover. Update Purpose after archive.
## Requirements
### Requirement: Magnetar Arena Delegation
Tachyon-Mesh SHALL delegate model tensor allocation, pinning, eviction, and VRAM/RAM ownership decisions to Magnetar's runtime memory arena.

#### Scenario: Loading a safetensors model
- **GIVEN** Tachyon has fetched a safetensors artifact for a supported Qwen model
- **WHEN** the model loader prepares the artifact for inference
- **THEN** Tachyon SHALL provide Magnetar with a memory-mapped artifact reference
- **AND** Magnetar SHALL return opaque tensor handles for runtime execution
- **AND** Tachyon host code SHALL NOT own raw tensor buffers after the handoff

### Requirement: Admission Capacity Check
Before accepting a local inference session, Tachyon-Mesh SHALL query Magnetar for the request's KV-cache admission capacity.

#### Scenario: Magnetar reports sufficient capacity
- **GIVEN** a node advertises a Magnetar provider capable of serving Qwen inference
- **AND** Magnetar reports enough contiguous capacity for the requested KV cache
- **WHEN** an inference request is admitted locally
- **THEN** Tachyon SHALL enqueue the request for Magnetar execution

#### Scenario: Magnetar reports OOM
- **GIVEN** a node advertises a Magnetar provider capable of serving Qwen inference
- **AND** Magnetar reports insufficient contiguous capacity for the requested KV cache
- **WHEN** an inference request targets that node
- **THEN** Tachyon SHALL reject the local request with a resource-exhaustion status
- **AND** the mesh router SHALL try another node with matching Qwen capabilities when one is available

