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

