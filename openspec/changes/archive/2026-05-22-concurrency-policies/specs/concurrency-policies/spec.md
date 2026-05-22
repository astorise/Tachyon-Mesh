## ADDED Requirements

### Requirement: integrity.lock accepts a concurrency policy on each route
The integrity schema SHALL accept an optional `concurrency` object on each route, declaring the execution concurrency mode and conflict handling policy.

#### Scenario: Default concurrency is unrestricted for backward compatibility
- **WHEN** a route is sealed without a `concurrency` field
- **THEN** the runtime treats it as `mode: "unrestricted"`, `on_conflict: "queue"`, `lock_ttl_ms: 30000`
- **AND** behavior is identical to pre-feature behavior

#### Scenario: NodeSingleton blocks concurrent invocations on the same node
- **WHEN** a route declares `concurrency: { mode: "node-singleton", on_conflict: "queue" }`
- **AND** an invocation is in progress on a node
- **AND** a second invocation arrives on the same node
- **THEN** the second invocation waits in a FIFO queue until the first completes
- **AND** invocations on other nodes are not affected

#### Scenario: MeshSingleton with on_conflict reject returns 409
- **WHEN** a route declares `concurrency: { mode: "mesh-singleton", on_conflict: "reject" }`
- **AND** an invocation holds the distributed lock anywhere in the mesh
- **AND** a second invocation arrives on any node
- **THEN** the second invocation returns HTTP 409 Conflict immediately
- **AND** the response body names the holding node

#### Scenario: MeshLeader rejects with redirect hint
- **WHEN** a route declares `concurrency: { mode: "mesh-leader" }`
- **AND** an invocation arrives on a non-leader node
- **THEN** the runtime returns HTTP 503 Service Unavailable
- **AND** an `X-Tachyon-Leader: <node-id>` response header points the client to the elected leader

### Requirement: integrity.lock accepts consistency modes on each volume
The integrity schema SHALL accept an optional `consistency` object on each volume, declaring how concurrent reads and writes are resolved.

#### Scenario: Default consistency preserves current behavior
- **WHEN** a volume is sealed without a `consistency` field
- **THEN** the runtime treats it as `read_mode: "snapshot"`, `write_mode: "last_write_wins"`
- **AND** behavior is identical to pre-feature behavior

#### Scenario: OptimisticEtag fails commit when remote ETag has changed
- **WHEN** an S3 volume declares `consistency: { write_mode: "optimistic_etag" }`
- **AND** two invocations run concurrently with the same initial state
- **AND** the first invocation commits successfully
- **WHEN** the second invocation attempts to commit
- **THEN** the conditional PUT fails because the ETag no longer matches
- **AND** the runtime logs a conflict warning and the affected invocation returns HTTP 409

#### Scenario: PessimisticLock serializes invocations sharing a volume
- **WHEN** a volume declares `consistency: { write_mode: "pessimistic_lock" }`
- **AND** two invocations targeting routes that mount this volume arrive concurrently
- **THEN** the second invocation waits for the first to release the lock before downloading the volume
- **AND** both invocations see consistent state (first's writes are visible to second's reads)

### Requirement: Distributed lock primitive backed by CoreStore
The platform SHALL provide a `DistributedLock` primitive in `core-host` backed by the embedded `CoreStore` (redb), with lease-based TTL and best-effort cross-node consistency via outbox sync.

#### Scenario: Lock acquisition is atomic within a node
- **WHEN** two concurrent local tasks call `DistributedLock::acquire(key, ttl)` on the same node
- **THEN** exactly one acquisition succeeds
- **AND** the other receives `LockError::Held { holder, expires_at }`

#### Scenario: Lock auto-expires after TTL
- **WHEN** a lock is acquired with `ttl: 5000ms`
- **AND** the holder crashes without releasing
- **AND** 5000 milliseconds elapse without a heartbeat refresh
- **THEN** a subsequent `acquire` call on any node succeeds

#### Scenario: Heartbeat extends the lock lease
- **WHEN** a lock is held with `ttl: 30000ms` and the holder calls `heartbeat()` every 15 seconds
- **THEN** the lock remains held indefinitely until the holder releases or stops heartbeats

### Requirement: Leader election by deterministic hashing of resource key
The platform SHALL provide a `leader_election::am_i_leader(resource_key)` function that returns true on exactly one node at any point in time, assuming all nodes share the same node registry view.

#### Scenario: Single mesh node is always leader
- **WHEN** the mesh has 1 active node
- **THEN** `am_i_leader("any-key")` returns true on that node

#### Scenario: Two nodes elect deterministically per resource
- **WHEN** the mesh has 2 active nodes `node-a` and `node-b`
- **AND** both nodes call `am_i_leader("route:/api/backup")` with identical node registry views
- **THEN** exactly one node returns true and the other returns false
- **AND** the result is deterministic across calls until the node registry changes
