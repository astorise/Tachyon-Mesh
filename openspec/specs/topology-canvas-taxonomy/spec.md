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

### Requirement: View / Edit mode toggle

`<tachyon-topology-panel>` SHALL expose a header toggle with two mutually exclusive states: **View** and **Edit**. The toggle MUST be rendered to the right of the panel title, on the same row as the existing zoom / compact / reset controls.

#### Scenario: Default mode on first mount is View

- **GIVEN** the operator opens the topology route for the first time in the current session
- **WHEN** `<tachyon-topology-panel>` mounts
- **THEN** the mode toggle reports `View` as selected
- **AND** the canvas renders the live graph (or the empty state) without any add-node form, "Apply Topology" button, or node-editor side panel visible

#### Scenario: Mode is persisted across reloads within the session

- **GIVEN** the operator switches the toggle to `Edit`
- **WHEN** the operator triggers a full page reload
- **THEN** the toggle is restored to `Edit` on remount
- **AND** the editing affordances are visible

#### Scenario: A new session defaults back to View

- **GIVEN** the operator switched to `Edit` and then closed the browser tab
- **WHEN** the operator opens a fresh tab and navigates to the topology route
- **THEN** the toggle reports `View` as selected
- **AND** `sessionStorage` is consulted exclusively (NOT `localStorage`)

### Requirement: Edit-mode-only affordances

In View mode, the following affordances SHALL be disabled or hidden:

- The "Add Node" form (`#add-node-form`) is hidden.
- The "Apply Topology" button (`#btn-apply-topology`) is hidden.
- Drag-to-move on nodes is disabled (pointer drag still pans the canvas).
- The wasm-drop target on the canvas is disabled (drop is rejected).
- The node-editor side panel does not open on node selection; clicking a node only highlights it.

#### Scenario: View mode hides edit controls

- **GIVEN** the panel is in View mode
- **WHEN** the panel renders
- **THEN** `#add-node-form` and `#btn-apply-topology` are not present in the DOM
- **AND** dragging a node has no visible effect on its `style.left` / `style.top`

#### Scenario: Edit mode restores every affordance

- **GIVEN** the operator switches to Edit mode
- **WHEN** the panel re-renders
- **THEN** `#add-node-form` and `#btn-apply-topology` are present
- **AND** dragging a node updates its position and dispatches `topology:node-moved`
- **AND** dropping a `.wasm` file dispatches `topology:wasm-dropped`

### Requirement: Mode is reflected in the source banner

The existing source banner (live / offline) SHALL append the current mode as a human-readable suffix.

#### Scenario: Live banner shows mode

- **GIVEN** `get_topology_graph` returned a live result and the panel is in `View` mode
- **WHEN** the panel renders the header
- **THEN** the banner text contains both the `topology.live-banner` content and a localised "View" suffix
- **AND** the suffix updates to "Edit" when the mode toggle is flipped

#### Scenario: Offline banner shows mode

- **GIVEN** `get_topology_graph` returned an empty graph and the panel is in `View` mode
- **WHEN** the panel renders the header
- **THEN** the banner text contains both the `topology.offline-banner` content and a localised "View" suffix

### Requirement: Topology empty state is covered by Playwright

The Playwright suite SHALL include a spec that asserts the empty-state card from `topology-canvas-taxonomy@2026-05-19` is rendered when `get_topology_graph` returns an empty graph, and that no `[data-node-id]` button exists in the canvas in that case.

#### Scenario: Empty backend produces empty canvas

- **GIVEN** the Tauri fake returns `{ nodes: [], edges: [], source: "live", status: "ok" }` from `get_topology_graph`
- **WHEN** the operator opens the topology route under Playwright
- **THEN** the empty-state card with the `topology.empty.title` label is visible
- **AND** zero elements matching `[data-node-id]` exist in the rendered shadow tree

### Requirement: Demo flag is covered by Playwright

The Playwright suite SHALL include a spec that asserts the `?demo=1` flag re-injects the sample graph and renders the "Demo data" banner.

#### Scenario: Demo flag restores sample graph

- **GIVEN** the Tauri fake returns an empty `get_topology_graph` response
- **AND** the application is loaded with `?demo=1` in the URL
- **WHEN** the operator opens the topology route under Playwright
- **THEN** at least one `[data-node-id]` element is present in the canvas
- **AND** a banner containing the `topology.demo.banner` text is rendered above the canvas

