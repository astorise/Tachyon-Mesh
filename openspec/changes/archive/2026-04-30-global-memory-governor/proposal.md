# Proposal: Global Memory Governor

## Context
Tachyon Mesh has multiple systems holding memory (Cwasm cache, Instance Pools, Rate Limiter maps, Request Buffers). Currently, they manage their limits independently. Under extreme load, their combined memory usage can exceed the physical host's RAM, triggering the Linux OOM Killer. 

## Proposed Solution
Introduce a `MemoryGovernor` in the `core-host` that monitors the actual physical memory used by the process (RSS - Resident Set Size) via `/proc/self/statm` (or OS equivalent).
- **Thresholds:** Define `soft_limit` (e.g., 75% RAM) and `hard_limit` (e.g., 90% RAM).
- **Global Backpressure Event:** When RSS crosses the `soft_limit`, the Governor broadcasts a `MemoryPressure::High` event. 
- **Component Response:** The Wasm Pool shrinks itself, the Cache evicts stale entries, and the Router Buffer starts rejecting requests with `503 Service Unavailable` instead of queuing them.

## Objectives
- Eliminate OOM (Out of Memory) crashes completely.
- Coordinate memory shedding gracefully across all isolated host components.