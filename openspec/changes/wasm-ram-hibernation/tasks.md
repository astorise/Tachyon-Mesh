# Implementation Tasks

## Phase 1: State Management
- [ ] Refactor the `core-host` caching mechanism (from the previous Instance Pooling change) to support the `InstanceState` enum.
- [ ] Ensure that every time a FaaS is invoked, its `last_accessed` timestamp is updated atomically.

## Phase 2: Snapshot and Disk I/O
- [ ] Create the `core-host/src/hibernation.rs` module.
- [ ] Implement the memory extraction logic to safely copy `wasmtime::Memory` into a binary file.
- [ ] Implement the background Tokio task that triggers the hibernation for modules idle > 300 seconds.

## Phase 3: The Wake-Up Flow
- [ ] Update the FaaS execution middleware to intercept calls to `Hibernated` instances.
- [ ] Implement the logic to read the snapshot file and inject it back into a freshly allocated Wasm instance.
- [ ] Ensure proper error handling: if the `.snap` file is corrupted or deleted by the OS, fallback to a standard Cold Start.

## Phase 4: Validation
- [ ] **Test Hibernation:** Start a Wasm module, wait 5 minutes. Verify via system monitoring (htop/activity monitor) that the `core-host` RAM usage drops significantly.
- [ ] **Test Thaw:** Send a request to the hibernated module. Verify the request succeeds and that the latency is between a Warm Start (<1ms) and a Cold Start (>100ms) — likely around 10-30ms due to NVMe read speeds.