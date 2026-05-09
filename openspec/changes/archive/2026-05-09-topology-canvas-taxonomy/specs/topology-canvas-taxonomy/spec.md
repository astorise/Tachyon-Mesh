# topology-canvas-taxonomy

## ADDED Requirements

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
Tachyon-UI SHALL expose a `<tachyon-topology-canvas>` web component
that accepts a `nodes` array and an `edges` array as data, renders
nodes as positioned `<div>` blocks, draws edges as SVG lines between
node anchors, and emits `topology:node-selected` when a node is
clicked.

#### Scenario: Canvas paints nodes from data
- **WHEN** the canvas is mounted with a `nodes` array containing
  positioned entries
- **THEN** every node is rendered at its declared coordinates
- **AND** every edge is rendered as an SVG line connecting two nodes

#### Scenario: Click selects a node and fires an event
- **WHEN** the operator clicks a node block
- **THEN** the canvas dispatches a `topology:node-selected` custom
  event whose `detail.nodeId` matches the clicked node identifier

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
