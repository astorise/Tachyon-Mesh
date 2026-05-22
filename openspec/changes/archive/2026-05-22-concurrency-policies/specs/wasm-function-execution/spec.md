## ADDED Requirements

### Requirement: Guest execution pipeline honors the route concurrency policy
The guest execution pipeline SHALL invoke a concurrency admission check before instantiating a guest, and SHALL block, reject, or proceed according to the route's `concurrency` policy.

#### Scenario: Unrestricted mode skips admission check
- **WHEN** a route declares `concurrency: { mode: "unrestricted" }`
- **THEN** every invocation proceeds directly to guest instantiation without acquiring any lock

#### Scenario: NodeSingleton admission release on guest completion
- **WHEN** a route declares `concurrency: { mode: "node-singleton" }`
- **AND** an invocation acquires the local admission slot
- **THEN** the slot is released as soon as the invocation completes (success, error, or panic)
- **AND** a queued invocation can immediately acquire the slot

#### Scenario: MeshSingleton heartbeats during long invocations
- **WHEN** a route declares `concurrency: { mode: "mesh-singleton", lock_ttl_ms: 10000 }`
- **AND** a guest invocation runs for longer than the TTL
- **THEN** the pipeline refreshes the distributed lock at TTL/2 intervals
- **AND** other nodes do not steal the lock during the long-running invocation
