# topology-live-and-filters Specification

## Purpose
TBD - created by archiving change topology-live-and-filters. Update Purpose after archive.
## Requirements
### Requirement: Topology reads live manifest
The topology graph SHALL be sourced from the live node's manifest
(`GET /admin/manifest`) when connected, not from the local `integrity.lock` file.
The `SealedConfig` deserialisation of the `IntegrityConfig` response MUST
correctly populate `routes`, `batch_targets`, `resources`, and `kv_caches`.

#### Scenario: Connected client sees live routes
- **WHEN** `get_topology_graph` is called while connected to a node
- **THEN** nodes reflect the routes currently active on the node, not the local file

#### Scenario: Offline fallback still works
- **WHEN** `get_topology_graph` is called with no active connection
- **THEN** nodes are sourced from the local `integrity.lock` as before

### Requirement: Two-tier node model for user WASM routes
The system SHALL emit two nodes and one edge for each user route whose `targets[0].module` is a WASM reference (`.wasm` suffix or `tachyon://` prefix):
- An `endpoint` node with the route's HTTP path as label and `protocol` in data.
- A `custom-wasm` node with the module name as label.
- An edge from endpoint → custom-wasm.

#### Scenario: Imported WASM route produces two nodes
- **WHEN** the live manifest contains `/api/guest-example` with `targets[0].module = "tachyon://sha256:…"`
- **THEN** the topology contains an endpoint node `route:/api/guest-example` and a custom-wasm node `wasm:guest-example` connected by an edge

#### Scenario: System route produces one node
- **WHEN** the live manifest contains `/system/logger` with `role: "system"`
- **THEN** only one `system-faas` node is emitted (no endpoint pair)

### Requirement: Collision-free two-pass layout
The topology layout engine SHALL compute base rows from actual per-type node
counts before assigning positions. A type's overflow sub-rows MUST NOT overlap
the base row of any subsequent type.

#### Scenario: Six system-faas nodes do not overlap custom-wasm band
- **WHEN** the topology contains 6 system-faas nodes and 4 custom-wasm nodes
- **THEN** all 6 system-faas nodes are placed at y=180 (rows 0-4) and y=320 (row 5)
- **THEN** custom-wasm nodes begin at y=460 (the next available band)

### Requirement: Manually-added nodes survive live reloads
`loadLiveTopology` SHALL merge live nodes with locally-added nodes. Nodes whose
`id` is not present in the live response MUST be preserved in the canvas. Live
edges replace their counterparts; manual edges between surviving manual nodes
are kept.

#### Scenario: User-drawn endpoint node preserved after reload
- **WHEN** an operator adds an `endpoint` node manually in edit mode
- **THEN** after `loadLiveTopology` completes, the manual node is still visible alongside the live nodes

### Requirement: Filter bar
The topology panel SHALL provide a filter bar with:
1. Text input filtering nodes by label (case-insensitive substring) and `data.tags`.
2. Type-chip buttons (one per `TopologyNodeType`) for multi-select type filtering.
3. Tag pill buttons auto-generated from `data.tags` fields across all nodes.
4. A checkbox to show or hide dependency edges independently of node filters.
5. An active-filter counter showing `Showing N / Total` and a *Clear filters* button.

#### Scenario: Text filter hides non-matching nodes
- **WHEN** the operator types "logger" in the filter input
- **THEN** only nodes whose label contains "logger" (or whose `data.tags` contains "logger") are visible on the canvas

#### Scenario: Type chip filters to a single type
- **WHEN** the operator clicks the `system-faas` chip
- **THEN** only system-faas nodes are visible; all other types are hidden

#### Scenario: Edge toggle hides all edges
- **WHEN** the operator unchecks *Show links*
- **THEN** no edges are drawn, regardless of other filters

#### Scenario: Clear button resets all filters
- **WHEN** active filters exist and the operator clicks *Clear filters*
- **THEN** all nodes and edges are visible again and the filter inputs are reset

### Requirement: Topology canvas allows drag in view mode
Topology canvas nodes SHALL be draggable in both view and edit modes. The
`editable` flag SHALL continue to gate node creation, deletion, the node-editor
sidebar, and the "Apply Topology" button, but SHALL NOT gate the node `pointerdown`
drag handler.

#### Scenario: Drag in view mode moves the node
- **WHEN** the topology is in view mode and the operator presses and drags a node
- **THEN** the node's position updates on the canvas in real time

#### Scenario: Node editor does not open in view mode
- **WHEN** the topology is in view mode and the operator clicks a node without dragging
- **THEN** the `topology:node-selected` event fires but the node-editor sidebar remains hidden (editable = false)

