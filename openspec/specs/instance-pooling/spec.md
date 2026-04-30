# instance-pooling Specification

## Purpose
Define bounded guest instance pooling, prewarming, and request wait-queue behavior for sealed
routes.
## Requirements
### Requirement: Targets are managed through bounded instance pools with prewarming and wait queues
The host SHALL prewarm a configurable minimum number of instances, enforce a configurable maximum
concurrency, and queue excess requests until capacity becomes available.

#### Scenario: Request load exceeds the available warm instances
- **WHEN** a target has exhausted its currently available instances but has not yet exceeded its
  maximum concurrency
- **THEN** the host creates or reuses pooled instances within the configured bounds
- **AND** places additional work into an in-memory wait queue until capacity returns

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

### Requirement: Cwasm cache entries are bound to Wasmtime compatibility
The host MUST include the active Wasmtime engine precompile compatibility hash in every serialized Cwasm cache key.

#### Scenario: Engine configuration changes
- **GIVEN** a Wasm module has already been cached as precompiled Cwasm
- **WHEN** the host restarts with a different Wasmtime version or engine configuration
- **THEN** the host computes a different compatibility hash
- **AND** it does not deserialize the stale Cwasm bytes with the new engine

### Requirement: Stale Cwasm cache is purged at boot
The host MUST compare the active engine compatibility hash with persisted cache metadata before loading any cached Wasm module.

#### Scenario: Stored compatibility hash differs from current engine
- **GIVEN** the cache metadata stores a previous engine compatibility hash
- **WHEN** the host boots with a different current compatibility hash
- **THEN** it clears the Cwasm cache bucket
- **AND** it stores the current compatibility hash before route prewarming can load cached Cwasm

### Requirement: Matching Cwasm cache is retained
The host MUST retain existing Cwasm cache entries when the persisted compatibility hash matches the current engine.

#### Scenario: Host restarts without engine changes
- **GIVEN** the cache metadata matches the active engine compatibility hash
- **WHEN** the host boots
- **THEN** it leaves Cwasm cache entries intact
- **AND** repeat module loads can reuse the cached precompiled bytes

