# wasm-bindings Specification

## Purpose
TBD - created by archiving change magnetar-full-cutover. Update Purpose after archive.
## Requirements
### Requirement: Opaque Tensor Handles
Tachyon-Mesh host bindings SHALL expose Magnetar tensor and kernel handles as opaque identifiers to WASM model components.

#### Scenario: WASM component executes a graph node
- **GIVEN** a WASM model component has a prepared kernel identifier
- **AND** the component has opaque input and output tensor identifiers
- **WHEN** it requests node execution
- **THEN** Tachyon SHALL pass only the identifiers to Magnetar
- **AND** Magnetar SHALL resolve the identifiers to concrete memory internally

### Requirement: No Raw Tensor Host Access
Tachyon-Mesh host bindings SHALL NOT expose APIs that read, write, or map raw tensor bytes by tensor identifier outside the model artifact I/O setup phase.

#### Scenario: Host binding registration is validated
- **GIVEN** Tachyon registers host functions for a WASM model component
- **WHEN** the binding set is inspected
- **THEN** no registered binding SHALL expose raw tensor byte reads
- **AND** no registered binding SHALL expose raw tensor byte writes
- **AND** execution SHALL rely on opaque tensor identifiers

