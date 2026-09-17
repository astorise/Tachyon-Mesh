# wasm-bindings Specification

## Purpose
Define the WASM inference host boundary after the Magnetar cutover. Tachyon exposes request/response bindings for local inference, while Magnetar owns tensor resources, prepared execution plans, Provider execution, and model memory internals.
## Requirements
### Requirement: WASM inference bindings MUST NOT expose Magnetar internals
Tachyon-Mesh host bindings SHALL NOT expose Magnetar tensor identifiers, prepared kernel identifiers, execution-plan handles, graph nodes, or Provider internals to WASM model components. WASM components SHALL submit inference requests through Tachyon's public inference contract and receive inference responses or stream events.

#### Scenario: WASM component invokes local inference
- **GIVEN** a WASM component submits a local inference request
- **WHEN** Tachyon routes the request to a local Magnetar-backed model
- **THEN** Tachyon SHALL pass prompt, generation, and routing intent through its inference adapter
- **AND** Magnetar SHALL own model ingestion, tensor resources, prepared execution, and Provider dispatch internally
- **AND** the WASM contract SHALL NOT contain `TensorId`, `PreparedKernelId`, tensor handle, layer execution, memory-profile, or topology-validation fields

### Requirement: No Raw Tensor Host Access
Tachyon-Mesh host bindings SHALL NOT expose APIs that read, write, or map raw tensor bytes by tensor identifier. Model artifact bytes MAY be transported or staged by Tachyon before Magnetar ingestion, but runtime tensors remain Magnetar-owned.

#### Scenario: Host binding registration is validated
- **GIVEN** Tachyon registers host functions for a WASM model component
- **WHEN** the binding set is inspected
- **THEN** no registered binding SHALL expose raw tensor byte reads
- **AND** no registered binding SHALL expose raw tensor byte writes
- **AND** execution SHALL rely on Magnetar's model runtime rather than Tachyon-minted tensor identifiers
