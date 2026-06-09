## ADDED Requirements

### Requirement: Distributed limiter exposes a check / merge / state HTTP surface
The `system-faas-dist-limiter` component SHALL expose three endpoints on its `handle-request` export. `POST /check` SHALL accept a JSON body `{ "key": string }`, increment the calling node's counter for that key, and return `{ "allowed": bool, "total": number }`, where `allowed` is `total <= DIST_LIMIT` (default 100). `POST /merge` SHALL accept a remote counter snapshot, fold it into local state, and return `204`. `GET /state` SHALL return the local counter snapshot as JSON. Any other route SHALL return `404`, and a malformed `/check` or `/merge` body SHALL return `400`.

#### Scenario: Check increments and reports the running total
- **WHEN** a `POST /check` arrives with `{ "key": "203.0.113.10" }`
- **THEN** the limiter increments the calling node's count for that key
- **AND** returns `{ "allowed": true, "total": 1 }` while the total is within `DIST_LIMIT`

#### Scenario: Over-limit checks report not allowed
- **WHEN** the summed total for a key exceeds `DIST_LIMIT`
- **THEN** `/check` returns `allowed: false` together with the current `total`

#### Scenario: State can be pulled for gossip
- **WHEN** a peer sends `GET /state`
- **THEN** the limiter returns its local per-key, per-node counter snapshot as JSON

### Requirement: Counters are keyed by a fixed time window
The limiter SHALL bucket counts under the composite key `{key}:{window}`, where `window` is `unix_seconds / DIST_LIMIT_WINDOW_SECONDS` (default 60). Counts in different windows SHALL be independent, so a key's allowance resets when the window rolls over.

#### Scenario: Window rollover resets the allowance
- **WHEN** the wall clock advances past a `DIST_LIMIT_WINDOW_SECONDS` boundary
- **THEN** new `/check` calls count against a fresh `{key}:{window}` bucket
- **AND** the previous window's total no longer constrains the new window

### Requirement: G-counter merge converges by per-node maxima
Each counter SHALL be a grow-only CRDT mapping `key -> { node_id -> count }` in which a node only ever increments its own entry and the effective total is the sum across nodes. Merging a remote snapshot SHALL take, for each `(key, node_id)`, the maximum of the local and remote value, so that merge is commutative, associative, and idempotent and all replicas converge regardless of gossip order.

#### Scenario: Merge takes the maximum per node
- **WHEN** node A holds `{ node-a: 2 }` for a key and merges node B's `{ node-a: 1, node-b: 1 }`
- **THEN** the merged counter is `{ node-a: 2, node-b: 1 }`
- **AND** the effective total is `3`

#### Scenario: Merge is order-independent
- **WHEN** two replicas exchange and merge each other's snapshots in either order
- **THEN** both converge to the same per-node maxima
- **AND** their effective totals are equal
