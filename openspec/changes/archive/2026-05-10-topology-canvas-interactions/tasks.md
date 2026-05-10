# Tasks

- [x] 1. Add `dragState` and `didDrag` fields to `TachyonTopologyCanvas`.
- [x] 2. Attach `pointerdown / pointermove / pointerup` listeners per node
       button; use `setPointerCapture` to track drags outside the button.
- [x] 3. Implement `updateEdgeSvgLive()` — updates SVG line attributes during
       drag without a full re-render.
- [x] 4. Emit `topology:node-moved` on drop; parent panel syncs coords.
- [x] 5. Add "Add Node" toolbar form to `TachyonTopologyPanel` with a type
       select and a label input; `addNode()` inserts at a random valid position.
- [x] 6. Add "Delete node" button to `TachyonNodeEditor`; emits
       `topology:node-delete`.
- [x] 7. Handle `topology:node-delete` in `TachyonTopologyPanel`: remove the
       node and all referencing edges, close the editor.
- [x] 8. Add 9 new i18n keys (EN + FR): add title/type/label/submit, delete,
       added/deleted feedback.
- [x] 9. Verify `npx tsc --noEmit` passes and `vite build` succeeds.
