## Context

The topology canvas distinguishes two modes — *view* (read-only display of live
data) and *edit* (add/move/delete nodes, apply topology). Drag was previously
gated on `this.editable` (set only in edit mode), which made the canvas
frustrating to explore: operators had to switch modes just to reorganise nodes
visually.

The layout bug was a consequence of the two-pass refactor: `pending_type_counts`
was incremented inline with a chained method call that rustfmt reformatted
differently on the CI runner, causing the format check to fail. The actual logic
was correct; only the line length triggered the linter.

## Goals / Non-Goals

**Goals:**
- Drag works in view mode without breaking the view/edit semantic for other
  features (node creation, deletion, sidebar editor).
- `system-faas-openai-adapter` appears in the topology automatically for
  `ai-inference` builds.
- `guest-examples.tar.gz` imports activate all 9 practical guest routes.
- CI passes with clean rustfmt output.

**Non-Goals:**
- Persisting dragged positions across page reloads (covered by a future change).
- Adding drag to the compact (bubble) mode if the pointer handling differs.

## Decisions

### D1 — Remove `editable` guard from `pointerdown` only
The `editable` flag is kept for: the `topology:wasm-dropped` handler (drag-to-
create), the "Add Node" form, the node-editor sidebar's save/delete actions, and
the "Apply Topology" button. Only the canvas node drag is ungated, because
repositioning nodes is a pure visual operation with no manifest side-effects.

### D2 — `ai-openai-adapter` alongside `ai-list-model` in injection bundle
The three AI routes (`model-broker`, `ai-list-model`, `ai-openai-adapter`) form
a cohesive feature unit: model storage, list API, and OpenAI-compatible adapter.
Injecting all three together makes the AI panel fully functional immediately
after deployment with `--features ai-inference`.

### D3 — Exclude tcp-echo, udp-echo, flaky, malicious from guest manifest
`guest-tcp-echo` and `guest-udp-echo` require Layer-4 route configuration that
doesn't fit the simple `path`/`role`/`name` manifest schema. `guest-flaky` is
a chaos-testing fixture. `guest-malicious` is a security test harness. All four
would confuse operators if accidentally activated in production.

## Risks / Trade-offs

- **Drag in view mode**: nodes moved in view mode update `this.nodes[i].{x,y}`
  but are not persisted. On the next `loadLiveTopology` call the live positions
  override them (unless the node is manual). This is acceptable: view mode is
  for exploration, edit mode for intentional layout changes.

## Open Questions

None.
