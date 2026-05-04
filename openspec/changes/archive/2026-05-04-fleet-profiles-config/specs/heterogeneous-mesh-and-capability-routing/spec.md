# heterogeneous-mesh-and-capability-routing Delta

## ADDED Requirements

### Requirement: Configuration updates MUST support targeted Fleet Profiles
The control plane SHALL NOT broadcast all configurations to all nodes. The `system-faas-gossip` and configuration broker SHALL use Node Selectors defined in `FleetProfile` schemas to selectively apply configurations to nodes matching specific metadata tags.

#### Scenario: Applying an optimized hardware profile to TPU nodes only
- **GIVEN** a fleet of nodes where only a subset have the tag `capabilities=tpu`
- **WHEN** the `system-faas-config-api` applies a `FleetProfile` targeting `capabilities=tpu` bound to a specific hardware configuration
- **THEN** only the nodes with the matching tag download and apply the TPU resource allocation
- **AND** standard nodes safely ignore the configuration event, preventing hardware capability mismatch panics.
