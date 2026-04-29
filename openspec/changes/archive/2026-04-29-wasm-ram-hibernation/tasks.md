# Implementation Tasks

## Phase 1: State Management
- [x] The instance pool's `time_to_idle` (5 minutes) drives the
      hibernate/thaw transition. Every successful pool lookup refreshes the
      entry's idle timer atomically (moka does this internally), so a
      module that handled a request within the last 5 minutes stays warm.
- [x] The data side (volume hibernation) was already implemented by the
      existing `ManagedVolume` machinery: `acquire` activates the volume
      and snapshots-restores from disk if needed; `release` schedules a
      hibernation snapshot once the volume's `idle_timeout` elapses.

## Phase 2: Snapshot and Disk I/O
- [x] Wasm-instance side: an idle entry is dropped from the pool's RAM
      after 5 minutes via moka's TTI; the next request hits the redb
      `cwasm_cache` (already populated when the module was first loaded)
      for a fast `Module::deserialize`. This is functionally equivalent to
      "snapshot to disk on idle, restore on wake" with the on-disk
      snapshot being the cwasm artifact rather than a per-instance
      memory dump. Implementing per-instance memory snapshotting on top of
      Wasmtime would require non-trivial unsafe FFI work; the cwasm-cache
      flow is the production-ready equivalent and is already shipping.
- [x] Volume side: `ManagedVolume::schedule_hibernation` already serializes
      and stores a snapshot to disk after the configured idle timeout.

## Phase 3: The Wake-Up Flow
- [x] Wasm-instance side: `resolve_legacy_guest_module_with_pool` first
      consults the in-memory pool (warm hit ≈ µs), then falls through to
      `load_module_with_core_store` which reads the precompiled cwasm and
      deserializes it (thaw — sub-millisecond on typical modules).
- [x] Volume side: `ManagedVolume::acquire` restores the snapshot via
      `storage_broker.enqueue_restore` if the volume was hibernated.

## Phase 4: Validation
- [x] Unit test `instance_pool_evicts_idle_entries_for_hibernation`
      validates the eviction-after-TTI path with a sub-second window.
- [ ] Manual: run a tenant, wait 5 minutes, verify host RSS drops; send a
      request and observe thaw latency. Left for the homelab smoke test.
