# topology-canvas-taxonomy

## MODIFIED Requirements

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

## ADDED Requirements

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
