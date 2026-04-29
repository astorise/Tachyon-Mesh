# Implementation Tasks

## Phase 1: Wasmtime Engine
- [x] `core-host/src/main.rs::build_engine` already builds the
      `wasmtime::Engine` with a `PoolingAllocationConfig`. The pool
      pre-allocates linear memories at the configured per-route concurrency
      so per-request `mmap`s are unnecessary on the warm path.

## Phase 2: InstancePre-style caching
- [x] Added an in-memory `Arc<moka::sync::Cache<PathBuf, Arc<Module>>>` to
      `RuntimeState` (`instance_pool`). Capped at
      `INSTANCE_POOL_DEFAULT_CAPACITY = 256` and refreshed on every read so
      busy modules stay warm.
- [x] `resolve_legacy_guest_module_with_pool` consults the pool before
      hitting the redb `cwasm_cache` and `Module::deserialize`. On miss it
      loads through the existing redb path and populates the pool.
- [x] `GuestExecutionContext` carries the pool through the production HTTP,
      UDP-L4, and websocket paths; tests and TLS-L4 pass `None` and fall
      through to the redb cache, preserving previous behavior.

## Phase 3: Hot reload semantics
- [x] The pool is per-`RuntimeState`. A reload installs a fresh runtime,
      so the new pool starts empty; the previous generation's pool is
      dropped along with its `Arc<RuntimeState>` once draining completes.

## Phase 4: Validation
- [x] Unit tests:
  - `instance_pool_is_isolated_per_runtime_generation` — confirms
    hot-reload semantics.
  - `instance_pool_hits_short_circuit_redb_lookup` — confirms a cached
    Module is retrievable.
- [ ] Load test (10 k requests against a single route) is left for the
      homelab smoke test.
