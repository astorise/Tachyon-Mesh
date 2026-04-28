# Implementation Tasks

## Phase 1: Wasmtime Engine Update
- [ ] Open `core-host` where the `wasmtime::Engine` is initialized (likely `core-host/src/main.rs` or a dedicated `store` module).
- [ ] Import `PoolingAllocationConfig` and apply it to the `wasmtime::Config`.
- [ ] Ensure the max instances and memory limits align with the router's expected resource constraints.

## Phase 2: InstancePre Caching
- [ ] Add a `moka::future::Cache` (or similar concurrent cache) to the `AppState` specifically for `InstancePre` objects.
- [ ] Refactor the module loading pipeline to stop calling `linker.instantiate` directly. Instead, compile the module, call `linker.instantiate_pre`, cache it, and then instantiate.

## Phase 3: FaaS Execution Refactoring
- [ ] Update the HTTP/3 and IPC routing middleware to leverage the new cached `InstancePre`.
- [ ] Ensure `Store` creation utilizes the updated `Engine`.

## Phase 4: Validation
- [ ] **Load Test:** Use a tool like `hey` or `wrk` to blast a simple FaaS endpoint with 10,000 requests. 
- [ ] Verify that CPU usage stays stable (proving compilation happens only once) and that the `core-host` RAM usage remains capped by the pool configuration.