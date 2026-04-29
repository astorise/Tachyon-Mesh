# distributed-crdt-rate-limiter Specification

## Purpose
TBD - created by archiving change distributed-crdt-rate-limiter. Update Purpose after archive.
## Requirements
### Requirement: Distributed limiter FaaS maintains CRDT-backed counters
The Mesh SHALL provide `system-faas-dist-limiter` which maintains per-IP request counters as G-Counters or LWW-Maps and broadcasts/merges the counter state across nodes via `system-faas-gossip`.

#### Scenario: Counters converge across nodes via gossip
- **WHEN** a route flagged with `distributed_rate_limit: true` receives traffic on multiple nodes
- **THEN** each node updates its local CRDT counter for the source IP
- **AND** counter deltas are propagated through `system-faas-gossip`
- **AND** all nodes eventually observe a consistent global request count for the IP within the configured window

### Requirement: Core host queries the distributed limiter only for opted-in routes and fails open on partition
The `core-host` SHALL query `system-faas-dist-limiter` over IPC for routes that explicitly set `distributed_rate_limit: true` in `integrity.lock`, and SHALL fall back to its local rate limiter if the FaaS is unavailable or the network is partitioned.

#### Scenario: Distributed limiter unavailable during partition
- **WHEN** a route flagged for distributed rate limiting receives a request
- **AND** `system-faas-dist-limiter` is unreachable or the gossip layer reports a partition
- **THEN** the host falls back to the local rate limiter for that request
- **AND** the request is accepted or rejected according to the local policy
- **AND** the host records a metric indicating that distributed enforcement was bypassed

