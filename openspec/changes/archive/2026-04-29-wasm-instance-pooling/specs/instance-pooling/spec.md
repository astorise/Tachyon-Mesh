## ADDED Requirements

### Requirement: Wasmtime engine is configured with a pooling allocator
The `core-host` SHALL initialize the `wasmtime::Engine` with a `PoolingAllocationConfig` that pre-allocates a fixed block of RAM for Wasm linear memories, so that per-request invocations do not trigger fresh OS-level allocations such as `mmap`.

#### Scenario: Cold-allocator path is not exercised on the hot path
- **WHEN** a configured Wasm module is invoked under steady-state load
- **THEN** the engine satisfies the linear memory request from the pre-allocated pool
- **AND** no new `mmap` syscall is observed for that invocation's linear memory
- **AND** the per-request allocator latency contribution is bounded and predictable

### Requirement: Modules are pre-compiled into InstancePre and cached
When a module is loaded from `system-faas-registry`, the host SHALL compile it exactly once into a `wasmtime::Module` and then build an `InstancePre`. These `InstancePre` values SHALL be retained in a concurrent LRU cache (e.g. `moka`) keyed by module identity.

#### Scenario: Repeat invocation hits the warm InstancePre cache
- **WHEN** a request targets a module already present in the `InstancePre` cache
- **THEN** the host fetches the cached `InstancePre`
- **AND** calls `.instantiate()` against the pooling allocator to obtain an instance
- **AND** the warm-start invocation latency stays below 1 ms on reference hardware

### Requirement: Memory used by the Wasm runtime is capped to a predictable limit
The pooling and pre-compilation configuration SHALL impose a hard cap on the total memory used by Wasm linear memories and pre-allocated structures, so the host's runtime memory footprint is predictable and bounded under bursty load.

#### Scenario: Concurrency surge does not exceed the configured cap
- **WHEN** the host receives a burst of concurrent requests targeting many distinct modules
- **THEN** the runtime memory used by Wasm linear memories does not exceed the configured pool size
- **AND** the host backpressures or rejects requests that would exceed the cap rather than allocating beyond it
