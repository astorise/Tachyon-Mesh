# global-memory-governor Specification

## Purpose
Define the centralized memory governor that prevents host-level OOM failures across Wasm pools, Cwasm caches, and request buffers.

## Requirements
### Requirement: RSS-aware pressure monitoring
The core host SHALL monitor resident memory pressure and classify it into normal, high, and critical states.

#### Scenario: Host enters critical memory pressure
- **WHEN** observed RSS exceeds the configured critical threshold
- **THEN** the memory governor broadcasts a critical pressure event
- **AND** subscribed components shrink caches, evict idle entries, or reject optional buffering work

### Requirement: Shared memory budget coordination
Memory-heavy components SHALL subscribe to governor events instead of enforcing only isolated limits.

#### Scenario: Cache pressure is high
- **WHEN** pressure is high but not critical
- **THEN** idle Wasm pool entries and cached modules are eligible for eviction before new allocations are denied
