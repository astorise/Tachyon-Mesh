## ADDED Requirements

### Requirement: Multi-master management API
Every healthy node MUST expose the secured management API so Tachyon UI can submit configuration updates to any reachable peer.

#### Scenario: UI submits config to a non-primary node
- **GIVEN** a mesh has multiple healthy nodes
- **WHEN** Tachyon UI submits a signed `integrity.lock` update to any node
- **THEN** that node validates the signature and version
- **AND** it applies the update locally through hot reload
- **AND** it initiates cluster-wide sync for peers

### Requirement: Gossip-triggered config notification
Nodes MUST announce accepted configuration updates through a lightweight gossip event containing version, checksum, and origin node identity.

#### Scenario: Peer receives newer config notification
- **GIVEN** a node has local configuration version 103
- **WHEN** it receives `tachyon.config.sync` for version 104
- **THEN** it compares the advertised version and checksum with local state
- **AND** it starts reconciliation from the origin node when the advertised version is newer

### Requirement: Secure pull reconciliation
Peers MUST pull the full manifest and missing module artifacts over the secure mesh overlay after receiving a newer config notification.

#### Scenario: Peer reconciles missing artifacts
- **GIVEN** a peer received a newer config notification
- **WHEN** it connects to the origin node through the secure mesh overlay
- **THEN** it downloads the new `integrity.lock`
- **AND** it fetches only missing or checksum-mismatched module artifacts
- **AND** it hot reloads after all integrity checks pass

### Requirement: Signed version conflict handling
Configuration conflicts MUST be resolved by signed monotonically increasing version metadata.

#### Scenario: Node receives stale signed config
- **GIVEN** a node already runs configuration version 104
- **WHEN** it receives a signed update for version 103
- **THEN** it rejects the stale update
- **AND** it keeps serving the current configuration
