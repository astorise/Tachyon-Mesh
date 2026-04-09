## ADDED Requirements

### Requirement: Mesh routing filters peers by declared capabilities before scoring load
The mesh router SHALL exclude nodes that cannot satisfy a route's required capabilities before comparing candidate peers on load or latency.

#### Scenario: A route requires capabilities not present on every node
- **WHEN** the router selects a peer for a capability-constrained workload
- **THEN** it filters out nodes lacking the required capabilities first
- **AND** only scores the remaining capable peers for final selection
