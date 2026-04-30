# Distributed Control Plane

## Purpose
Define how Tachyon keeps cluster steering and overflow policy in system control-plane functions while the host exposes only telemetry, route mutation, and replay-safe forwarding primitives.
## Requirements
### Requirement: Cluster steering logic runs in system control-plane functions rather than the host data plane
The host SHALL expose telemetry and route-update capabilities while system control-plane functions perform gossip, overflow decisions, and buffering policy.

#### Scenario: Local pressure exceeds the healthy threshold
- **WHEN** control-plane logic observes rising local saturation and a healthier peer is available
- **THEN** it updates routing decisions through host capabilities instead of embedding that policy in the host request path
- **AND** can redirect traffic to a buffer route when the entire cluster is saturated

#### Scenario: Buffered requests replay once local pressure recovers
- **GIVEN** traffic has been redirected to a control-plane buffer route
- **WHEN** the buffer system function observes that local pressure has returned below its replay thresholds
- **THEN** it replays persisted requests to their original mesh routes
- **AND** the host bypasses control-plane overrides for replay-marked requests to avoid infinite buffering loops

### Requirement: Every core-host exposes the same management API
Every `core-host` instance SHALL expose the full management API (the endpoints used by Tachyon Studio to update `integrity.lock` and Wasm modules), gated by the same admin credentials, so that Tachyon UI can connect to any node to administer the mesh.

#### Scenario: UI connects to an arbitrary node and applies a configuration
- **WHEN** an administrator points Tachyon Studio at any reachable mesh node and submits a new configuration
- **THEN** the receiving node validates the request against the admin credentials
- **AND** updates its local state and increments the global `config_version`
- **AND** acknowledges the update to the UI without forwarding to a designated master

### Requirement: Configuration updates propagate via gossip-triggered pull
Upon accepting a configuration update, a node SHALL broadcast a `ConfigUpdateEvent { version, checksum, origin_node_id }` over `system-faas-gossip`. Peer nodes SHALL pull the full `integrity.lock` and any missing Wasm binaries from the originating node over the `system-faas-mesh-overlay` secure tunnel, but only when the advertised version is newer than their own.

#### Scenario: Peer pulls a newer config from origin
- **WHEN** a peer receives a `ConfigUpdateEvent` with a version higher than its current `config_version`
- **THEN** the peer pulls the new `integrity.lock` and any missing Wasm binaries from the origin node over the secure overlay tunnel
- **AND** verifies the administrative signature on the configuration
- **AND** applies the update locally, advancing its `config_version` to match the origin
- **AND** peers that already have the same or a newer version do not initiate a pull

### Requirement: Configuration updates require an administrative signature
The mesh SHALL refuse to apply any configuration update whose payload is not signed by an administrative private key trusted by the cluster, regardless of the originating node.

#### Scenario: Unsigned config update is rejected
- **WHEN** a node receives a `ConfigUpdateEvent` whose pulled payload is not signed by a trusted administrative key
- **THEN** the node refuses to apply the update
- **AND** logs a security event identifying the originating node
- **AND** retains its previous configuration unchanged

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

