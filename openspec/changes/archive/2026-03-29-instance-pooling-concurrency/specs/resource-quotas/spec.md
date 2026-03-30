## ADDED Requirements

### Requirement: Shared Wasmtime engine uses pooled instance allocation
The `core-host` runtime SHALL configure the shared Wasmtime engine with the pooling allocator so
guest execution reuses reserved instance capacity instead of relying solely on on-demand
allocation.

#### Scenario: Engine is configured for pooled guest allocation
- **WHEN** the host builds the shared Wasmtime engine during startup
- **THEN** fuel metering remains enabled
- **AND** component-model support remains enabled
- **AND** the engine uses `PoolingAllocationConfig` sized from the sealed route concurrency plus
  the existing guest memory ceiling
