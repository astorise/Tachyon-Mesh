# Proposal: Topology Canvas Interactions — Drag, Add, Delete

## Why

The initial topology-canvas-taxonomy delivery shipped `<tachyon-topology-canvas>`
with static, fixed-position nodes. Operators could click a node to open the
editor but could not reposition nodes, add new ones, or remove them. The canvas
was effectively read-only beyond property editing.

## What Changes

- **Drag-and-drop.** Nodes respond to `pointerdown` / `pointermove` / `pointerup`
  events. The pointer is captured on the node button so drag works outside the
  button bounds. SVG edges update live during the drag via direct attribute
  manipulation (no full re-render). On `pointerup` the final position is committed
  to `this.nodes` and a `topology:node-moved` event is emitted; the parent panel
  updates its state without re-rendering.
- **Add Node.** A toolbar form below the canvas exposes a type selector and a
  label field. Submitting it creates a new node at a random position within the
  visible canvas area and pushes the graph to the canvas.
- **Delete Node.** The `<tachyon-node-editor>` gains a "Delete node" button that
  emits `topology:node-delete`. The parent panel removes the node and all edges
  that reference it, then closes the editor.
- All new user-visible strings are sourced through `t()` with EN + FR dictionary
  entries.
