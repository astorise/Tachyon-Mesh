# storage-fleet Specification Delta

## ADDED Requirements

### Requirement: Subspace Access Tracker
The core host SHALL track local and remote access counts by key subspace and peer so hot remote access patterns can trigger migration planning without storing every individual key.

#### Scenario: Remote peer dominates a subspace
- **WHEN** remote hits for a subspace exceed the configured minimum and ratio over local hits
- **THEN** the tracker emits a migration plan targeting that peer

### Requirement: Zero-Downtime Migration Planning
The mesh migration layer SHALL represent a migration as a subspace and target peer plan that can be executed by the QUIC replication path without blocking normal reads.

#### Scenario: Migration plan is produced
- **WHEN** the access tracker crosses the migration threshold
- **THEN** the plan identifies the subspace and new target peer

### Requirement: Gossip Route Update Table
The mesh SHALL maintain a subspace routing table that accepts route updates and resolves the current primary peer for a key by longest matching subspace prefix.

#### Scenario: More specific subspace exists
- **WHEN** both `tenant` and `tenant/users` have primary peers
- **THEN** a key under `tenant/users` resolves to the more specific primary peer

### Requirement: Geo-Pinning Cooldown
The migration tracker SHALL apply a cooldown after emitting a migration plan so the same subspace cannot flap repeatedly between peers.

#### Scenario: Cooldown is active
- **WHEN** a subspace has just emitted a migration plan
- **THEN** repeated accesses during the cooldown do not emit another plan
