# distributed-crdt-rate-limiter Specification

## Purpose
Define the distributed CRDT-backed rate limiter and its identity-aware scoping model.

## Requirements
### Requirement: Identity-scoped distributed counters
The rate limiter SHALL maintain CRDT counters that can be scoped by source IP, tenant identity, or token identity.

#### Scenario: Tenant scoped route limit
- **WHEN** a route configures tenant-scoped rate limiting
- **THEN** the limiter groups requests with keys using the `tenant:{tenant_id}:{route}` format
- **AND** requests from different tenants do not consume the same distributed counter

### Requirement: Bounded bypass behavior
The host SHALL fail open only within the configured distributed-rate-limit timeout when remote CRDT state is unavailable.

#### Scenario: CRDT peer does not respond
- **WHEN** distributed state lookup exceeds the timeout
- **THEN** the request path records a bypass metric
- **AND** the route continues using local enforcement instead of blocking indefinitely
