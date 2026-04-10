## ADDED Requirements

### Requirement: Mesh overflow decisions account for hardware queue depth and QoS class
The router SHALL use both QoS class and per-accelerator queue pressure when deciding whether to keep work local, overflow to a peer, or buffer it.

#### Scenario: A real-time request encounters local hardware backlog
- **WHEN** a real-time workload targets a hardware queue with measurable local backlog and a healthier peer is available
- **THEN** the router prefers remote overflow sooner than it would for batch work
- **AND** preserves more conservative local execution behavior for lower-priority workloads
