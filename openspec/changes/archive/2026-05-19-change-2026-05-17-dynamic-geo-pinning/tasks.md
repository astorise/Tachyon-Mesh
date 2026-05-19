# Implementation Tasks

- [x] **Task 1: Probabilistic Tracker**
  - Integrate a `CountMinSketch` (or similar memory-bound probabilistic data structure) in the `core-host` RedDB request handler to track remote vs. local access frequencies per K/V subspace.

- [x] **Task 2: Background Migration Engine**
  - Implement `core-host/src/mesh/migration.rs`.
  - Wire the migration trigger to initiate the 3-phase zero-downtime replication over the existing QUIC transport layer.

- [x] **Task 3: Gossip Routing Updates**
  - Update the Gossip protocol (`core-host/src/mesh/gossip.rs`) to support the `UpdateRoute` message type.
  - Ensure the internal `system-faas-gateway` uses this updated routing table to forward database reads/writes to the correct primary node.

- [x] **Task 4: Anti-Flapping Safeguards**
  - Implement a cooldown mechanism (e.g., a timestamp lock in the routing table) to prevent a subspace from continuously bouncing back and forth between two nodes competing for access.
