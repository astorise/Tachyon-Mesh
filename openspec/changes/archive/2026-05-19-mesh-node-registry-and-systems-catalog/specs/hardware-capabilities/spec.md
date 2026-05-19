## ADDED Requirements

### Requirement: Mesh-wide hardware capability retrieval

The host SHALL expose a mesh-wide hardware capability retrieval surface in addition to the existing local-only `hardware://local/status` resource. A caller MUST be able to retrieve the declared `NodeCapabilities` of any enrolled node, identified by its `node_id`, through the mesh node registry rather than scanning the local process.

#### Scenario: MCP client reads remote node status

- **GIVEN** an MCP client is connected to `tachyon-mcp`
- **AND** node `A` is enrolled and has reported its capabilities
- **WHEN** the client reads `hardware://mesh/A/status`
- **THEN** the server returns node `A`'s `NodeCapabilities` payload (RAM, accelerators, per-GPU stats, region)
- **AND** the response is sourced from the mesh node registry, not from the local process

#### Scenario: Unknown node id returns a structured not-found

- **WHEN** the MCP client reads `hardware://mesh/does-not-exist/status`
- **THEN** the server returns a structured error indicating the node is not enrolled
- **AND** the error does not leak the registry's internal storage layout

#### Scenario: Awaiting-capabilities node returns explicit awaiting status

- **GIVEN** node `B` is enrolled but has not yet reported its capabilities
- **WHEN** the MCP client reads `hardware://mesh/B/status`
- **THEN** the server returns a payload with `status = "awaiting-capabilities"` and no hardware fields
- **AND** the response is distinguishable from a populated payload with zero values

### Requirement: Cluster-wide hardware summary

The host SHALL expose a summary resource that aggregates the per-node hardware capabilities into a cluster-level view, intended to power the Tachyon-UI Overview "nodes" KPI.

#### Scenario: Cluster summary lists enrolled nodes

- **GIVEN** three nodes are enrolled and have reported capabilities
- **WHEN** a client reads the cluster-level summary resource
- **THEN** the response contains an `enrolled_count` of 3
- **AND** the response contains the sum of total RAM across the three nodes
- **AND** the response lists the distinct accelerator kinds seen across the fleet

#### Scenario: Cluster summary excludes nodes still awaiting capabilities

- **GIVEN** two nodes have reported capabilities and one is `awaiting-capabilities`
- **WHEN** a client reads the cluster summary
- **THEN** `enrolled_count` is 3
- **AND** an `awaiting_count` field reports the value 1
- **AND** the aggregated RAM and accelerator values only include the two reporting nodes
