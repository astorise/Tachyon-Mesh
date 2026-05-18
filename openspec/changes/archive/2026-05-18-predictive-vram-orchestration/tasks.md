# Implementation Tasks

- [ ] **Task 1: Priority VRAM Allocation**
  - Update the `core-host` `InstanceInferenceState` and Candle memory mappers to support the `VramPriority` enum.
  - Implement the instant-eviction logic for `Volatile` tensors to guarantee hardware safety.

- [ ] **Task 2: JIT Listener Component**
  - Update the `system-faas-model-broker` to subscribe to the CDC stream emitted by `system-faas-authn` when a new JWT/Biscuit session is issued.
  - Wire the broker to call the `load-layer` WIT function with the volatile flag.

- [ ] **Task 3: Heuristic TTL Querying**
  - Create a lightweight statistical function within the broker that queries the `system-faas-timeseries` component to calculate the probability of subsequent requests based on the tenant's historical access patterns.
  - Apply this dynamic TTL when a user finishes a prompt.
