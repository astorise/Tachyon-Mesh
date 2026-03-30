# resource-quotas Specification

## Purpose
Define deterministic CPU and memory limits for guest execution so `core-host` can contain malicious or runaway workloads without crashing.
## Requirements
### Requirement: Wasmtime engine enables deterministic fuel accounting
The `core-host` runtime SHALL initialize Wasmtime with fuel consumption enabled so every guest execution can be bounded by instruction count.

#### Scenario: Engine is configured for fuel metering
- **WHEN** the host creates the shared Wasmtime engine for guest execution
- **THEN** the engine configuration enables fuel consumption
- **AND** guest modules are instrumented so instruction usage can be decremented during execution

### Requirement: Each request applies CPU and memory quotas to the guest store
For every HTTP request, `core-host` SHALL create a fresh `Store`, inject a bounded fuel budget before invoking the guest entrypoint, and enforce a memory ceiling of 50 MiB for guest linear memory growth.

#### Scenario: Request-scoped store receives hard resource limits
- **WHEN** the host prepares a store for a guest request
- **THEN** the store receives a finite fuel allocation before guest code runs
- **AND** the store enforces a 50 MiB maximum linear memory budget for the guest
- **AND** those limits apply only to that request's guest execution

### Requirement: Host degrades gracefully when a guest exhausts quotas
If guest execution traps because it exhausts fuel or memory, `core-host` SHALL log a warning and return an HTTP `500 Internal Server Error` response with a body explaining that the resource limit was exceeded, without crashing the host process.

#### Scenario: Malicious guest is trapped without crashing the host
- **WHEN** a guest enters an infinite loop or attempts to exceed the configured memory budget
- **THEN** Wasmtime aborts the guest execution with a trap
- **AND** the host logs a warning describing the trapped execution
- **AND** the HTTP response is `500 Internal Server Error` with the text `Execution trapped: Resource limit exceeded`
- **AND** the host remains available to serve subsequent requests

### Requirement: Workspace provides a malicious guest test vector
The workspace SHALL include a `guest-malicious` WASI crate that intentionally exercises the quota limits by looping indefinitely or attempting excessive allocation.

#### Scenario: Malicious test guest exists for isolation verification
- **WHEN** a developer inspects the workspace members for the quota change
- **THEN** a `guest-malicious` crate is present as a WASI guest module
- **AND** its exported behavior is designed to trigger the configured fuel or memory limits during host execution

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

