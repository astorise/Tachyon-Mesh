# Implementation Tasks
- [x] Implement cross-platform RSS fetching through the existing `sysinfo` dependency.
- [x] Create the `MemoryGovernor` background task and its shared pressure state.
- [x] Wire warm Wasmtime/cwasm instance-pool entries to evict under pressure.
- [x] Wire the `system-faas-buffer` to stop accepting new requests during `Critical` pressure.
