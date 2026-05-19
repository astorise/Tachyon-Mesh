# topology-canvas-taxonomy Specification

## Purpose
TBD - created by archiving change topology-canvas-taxonomy. Update Purpose after archive.
## Requirements
### Requirement: Visual Node Taxonomy
Tachyon-UI SHALL render every topology node through one of eight visually
distinct profiles, each with its own Tailwind color palette, icon hint,
and badge slot. The eight types SHALL be `endpoint`, `system-faas`,
`custom-wasm`, `llm`, `kv-cache`, `storage`, `message-broker`, and
`external-resource`.

#### Scenario: Each node type renders with its assigned theme
- **WHEN** the canvas renders a node of type `llm`
- **THEN** the node is rendered with the fuchsia palette
  (`bg-fuchsia-900/40 border-fuchsia-500/80`)
- **AND** the badge slot displays the `modelName` value when set

#### Scenario: Unknown node type falls back to a neutral profile
- **WHEN** the canvas renders a node whose `type` is not one of the
  eight known values
- **THEN** the node is rendered with the slate fallback palette
- **AND** the rendering does not crash the canvas

### Requirement: Topology Canvas Web Component
The `<tachyon-topology-canvas>` component SHALL support pointer-based
drag-and-drop repositioning of nodes, updating SVG edges in real time
during the drag and committing the final position via a
`topology:node-moved` event on release.

#### Scenario: Drag repositions a node
- **WHEN** the operator presses and holds on a node then moves the pointer
- **THEN** the node follows the pointer within the canvas bounds
- **AND** the SVG edges connecting that node update their endpoints live
- **AND** releasing the pointer emits `topology:node-moved` with the final
  coordinates

#### Scenario: Click after drag does not select
- **WHEN** the operator drags a node and releases
- **THEN** the subsequent implicit click event is suppressed
- **AND** no `topology:node-selected` event is dispatched for that release

### Requirement: Contextual Node Editor
Tachyon-UI SHALL expose a `<tachyon-node-editor>` web component that
slides in from the right edge when a node is selected and renders a
form whose fields depend on the selected node's `type`. At minimum it
SHALL handle `llm`, `kv-cache`, `external-resource`, and `custom-wasm`
with the field set documented in the design notes.

#### Scenario: Editor renders LLM-specific fields
- **GIVEN** a node of type `llm` is selected
- **WHEN** `<tachyon-node-editor>` opens
- **THEN** the form exposes a model-name input, a quantization select
  (`INT4`, `INT8`, `FP16`), and a LoRA mode select (`dynamic`,
  `static`)

#### Scenario: Editor commits changes back to the canvas
- **WHEN** the operator submits the editor form
- **THEN** the editor dispatches a `topology:node-updated` event whose
  `detail.node` carries the updated node object
- **AND** the canvas re-renders the affected node with the new badge
  values

### Requirement: Graph Serialization
The canvas state SHALL be serializable through a `topology:serialize`
event that emits a JSON object containing a `nodes` array and an
`edges` array, each entry annotated with its WIT-aligned domain hint
(routing, ai, storage, supply-chain, etc.).

#### Scenario: Serialize event carries the full graph
- **WHEN** the operator triggers the "Build Bundle" control
- **THEN** the canvas dispatches `topology:serialize`
- **AND** the event detail contains both `nodes` and `edges` arrays
- **AND** every node entry carries the `type` it was rendered with

### Requirement: Add-Node Toolbar
`<tachyon-topology-panel>` SHALL expose a form with a node-type selector
and a label input that, when submitted, inserts a new node at a random
position inside the visible canvas area.

#### Scenario: Add node with type and label
- **GIVEN** the operator selects a type and enters a label in the toolbar
- **WHEN** they submit the add form
- **THEN** a new node with a unique id appears on the canvas at a random
  position
- **AND** `topology.feedback.added` is displayed in the feedback zone

### Requirement: Delete Node from Editor
`<tachyon-node-editor>` SHALL expose a "Delete node" button that emits
`topology:node-delete`. The parent panel SHALL remove the node and all
edges that reference it and close the editor.

#### Scenario: Delete removes node and connected edges
- **GIVEN** a node is selected and the editor is open
- **WHEN** the operator clicks "Delete node"
- **THEN** the node is removed from the canvas
- **AND** all edges that had that node as `from` or `to` are also removed
- **AND** the editor closes

### Requirement: Honest empty state when the backend reports no nodes

`<tachyon-topology-panel>` SHALL NOT render the previous `DEFAULT_NODES` / `DEFAULT_EDGES` sample graph as a fallback when `get_topology_graph` returns an empty node list. Instead, the panel SHALL render an empty-state card that clearly states no topology data was received and that links the operator to the new `nodes` view.

#### Scenario: Empty backend response shows empty state, not sample data

- **GIVEN** `get_topology_graph` returns `{ nodes: [], edges: [], source: "live", status: "ok" }`
- **WHEN** `<tachyon-topology-panel>` mounts
- **THEN** the canvas area renders an empty-state card with the message "No topology data received from the mesh"
- **AND** the card contains a link or button that navigates to the `nodes` route
- **AND** no `<button data-node-id>` element exists in the rendered canvas

#### Scenario: Demo flag re-enables sample graph for development only

- **GIVEN** the application is loaded with `?demo=1` in the URL
- **WHEN** `<tachyon-topology-panel>` mounts and `get_topology_graph` returns an empty node list
- **THEN** the panel loads the sample graph from `topology.demo.ts` and renders it as before
- **AND** an unmistakable banner reading "Demo data — not connected to a real mesh" is rendered above the canvas

### Requirement: Live vs offline source banner remains accurate

The existing live / offline banner SHALL reflect whether `get_topology_graph` returned non-empty data, regardless of whether the panel is currently displaying the demo fallback under `?demo=1`.

#### Scenario: Offline banner is shown for empty response without demo flag

- **GIVEN** `get_topology_graph` returns an empty node list and `?demo=1` is not set
- **WHEN** the panel renders
- **THEN** the header displays the offline banner styled with `text-amber-400/70`
- **AND** the canvas area shows the empty-state card defined above

#### Scenario: Demo banner overrides live banner under ?demo=1

- **GIVEN** `?demo=1` is set and the panel falls back to the sample graph
- **WHEN** the panel renders
- **THEN** the header MUST NOT display a `topology.live-banner` text claiming the data is live
- **AND** the demo banner from the previous requirement is the only banner shown

