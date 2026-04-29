## ADDED Requirements

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
