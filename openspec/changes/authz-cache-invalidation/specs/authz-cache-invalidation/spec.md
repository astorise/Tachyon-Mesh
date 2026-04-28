## ADDED Requirements

### Requirement: Authorization mutations emit cache purge events
`system-faas-authz` SHALL emit a targeted `authz.cache.purge` event whenever it processes a mutation that affects authorization decisions (token revocation, role update, user ban), including identifying fields such as `token_hash` or `user_id`.

#### Scenario: Token revocation publishes a purge event
- **WHEN** an administrator revokes a Personal Access Token via `system-faas-authz`
- **THEN** `system-faas-authz` emits an `authz.cache.purge` event referencing the affected `token_hash`
- **AND** the event is published on the internal event bus before the mutation acknowledgement is returned to the caller

### Requirement: Core host evicts cached authorization decisions on purge
The `core-host` SHALL maintain a background subscriber on the `authz` event channel and, upon receiving a purge event, immediately evict the matching entries from its in-process RBAC cache.

#### Scenario: Purge event invalidates the host RBAC cache
- **WHEN** the host's authz subscriber receives an `authz.cache.purge` event for a specific `token_hash`
- **THEN** the host removes any entries keyed by that `token_hash` from its local cache
- **AND** the next request presenting that token re-checks authorization against `system-faas-authz`
- **AND** the rejection happens within sub-second latency of the original mutation
