## ADDED Requirements

### Requirement: Dedicated Nodes route

Tachyon-UI SHALL expose a `nodes` route registered in `tachyon-ui/src/registry/ComponentRegistry.ts` that mounts a `<tachyon-nodes-panel>` custom element. The route MUST appear in the sidebar navigation above the existing `fleet` (policy) route.

#### Scenario: Nodes route is listed in navigation

- **WHEN** the operator opens the application shell after authentication
- **THEN** the sidebar contains a "Nodes" entry that activates the `nodes` route
- **AND** the corresponding panel mounts under `<tachyon-nodes-panel>`

### Requirement: Node inventory table

`<tachyon-nodes-panel>` SHALL fetch the enrolled-node list via the `list_enrolled_nodes` Tauri command on mount and render it as a table with at minimum: node id, status, last seen, total RAM (MiB), number of GPUs, and accelerators (comma-joined).

#### Scenario: Inventory renders the registry contents

- **GIVEN** `list_enrolled_nodes` returns three entries with mixed statuses
- **WHEN** the panel mounts
- **THEN** the table contains exactly three rows
- **AND** each row displays the corresponding `node_id`, `status`, `last_seen`, RAM, GPU count, and accelerators
- **AND** rows with `status = "stale"` MUST be visually distinguished (amber badge) from `status = "online"` (cyan badge)

#### Scenario: Empty registry shows guidance, not a placeholder row

- **GIVEN** `list_enrolled_nodes` returns an empty array
- **WHEN** the panel mounts
- **THEN** the table is hidden
- **AND** an empty-state block instructs the operator on how to enrol their first node
- **AND** the empty-state block links to the existing operator-invite generator

### Requirement: Per-node capability drill-in

The panel SHALL allow the operator to click a row and see the node's full `NodeCapabilities` payload, including per-GPU VRAM totals and usage. The drill-in MUST request the data via the `get_node_capabilities` Tauri command rather than reading from the in-memory list.

#### Scenario: Drill-in fetches fresh capabilities

- **GIVEN** the inventory shows three nodes
- **WHEN** the operator clicks node `A`
- **THEN** the UI invokes `get_node_capabilities("A")`
- **AND** the resulting payload is rendered in a side panel
- **AND** the side panel includes a per-GPU breakdown table with model, VRAM total, VRAM used, and utilisation percentage

#### Scenario: Awaiting-capabilities node renders explicit placeholder

- **GIVEN** node `B` was just approved and has `status = "awaiting-capabilities"`
- **WHEN** the operator clicks node `B`
- **THEN** the side panel displays "Awaiting first capability report" rather than zero-valued fields
- **AND** the panel does not invoke `get_node_capabilities` (or invokes it and shows the same placeholder if the result is empty)

### Requirement: Refresh control

The panel SHALL expose a "Refresh" control that re-fetches `list_enrolled_nodes` without re-mounting the panel. The control MUST be debounced to at most one call per 1500 ms.

#### Scenario: Refresh updates the table

- **GIVEN** the inventory shows two nodes
- **WHEN** a third node is approved and the operator clicks "Refresh"
- **THEN** the table re-renders with three rows
- **AND** the panel does not flash an empty-state block during the re-render
