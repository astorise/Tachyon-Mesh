# Heterogeneous Mesh And Capability Routing

## Purpose
Define how Tachyon advertises static node capabilities and filters mesh overflow candidates so routes are only forwarded to peers that can actually execute the requested workload.
## Requirements
### Requirement: Mesh routing filters peers by declared capabilities before scoring load
The mesh router SHALL exclude nodes that cannot satisfy a route's required capabilities before comparing candidate peers on load or latency.

#### Scenario: A route requires capabilities not present on every node
- **WHEN** the router selects a peer for a capability-constrained workload
- **THEN** it filters out nodes lacking the required capabilities first
- **AND** only scores the remaining capable peers for final selection

### Requirement: Incapable local nodes fail fast when no capable peer is available
The host SHALL reject capability-constrained requests with a clear service-unavailable response when the local node cannot execute them and no capable peer is known.

#### Scenario: No mesh peer satisfies the route requirements
- **WHEN** a request targets a route whose required capabilities are absent on the local host and on all known peers
- **THEN** the host returns `503 Service Unavailable`
- **AND** the response explains which capabilities are missing

### Requirement: Configuration updates MUST support targeted Fleet Profiles
The control plane SHALL NOT broadcast all configurations to all nodes. The `system-faas-gossip` and configuration broker SHALL use Node Selectors defined in `FleetProfile` schemas to selectively apply configurations to nodes matching specific metadata tags.

#### Scenario: Applying an optimized hardware profile to TPU nodes only
- **GIVEN** a fleet of nodes where only a subset have the tag `capabilities=tpu`
- **WHEN** the `system-faas-config-api` applies a `FleetProfile` targeting `capabilities=tpu` bound to a specific hardware configuration
- **THEN** only the nodes with the matching tag download and apply the TPU resource allocation
- **AND** standard nodes safely ignore the configuration event, preventing hardware capability mismatch panics.

