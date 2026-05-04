# p2p-mesh-overlay Delta

## ADDED Requirements

### Requirement: Gossip parameters MUST be dynamically tunable
The `system-faas-gossip` component SHALL update its internal ticker loops (heartbeats and timeouts) dynamically in response to changes in the GitOps `topology-configuration` without dropping existing peer connections.

#### Scenario: Tuning gossip timeouts during network degradation
- **WHEN** the `system-faas-config-api` receives a configuration increasing the `peer_timeout_ms` from 3000 to 10000
- **THEN** the gossip broker hot-reloads the configuration
- **AND** delays the eviction of unresponsive nodes to accommodate the new 10-second tolerance window.
