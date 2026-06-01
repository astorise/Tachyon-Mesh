## ADDED Requirements

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
