## Context

`get_topology_graph` in `tachyon-client` calls `get_admin_json::<SealedConfig>(ADMIN_MANIFEST_PATH)`.
`SealedConfig` uses `#[serde(default)]` on all fields, so when the server
returned the `{config_payload, …}` wrapper format (pre-fix), serde silently
produced an empty struct and the function fell back to the local file.
After the manifest-format fix (see `import-faas-package`), the GET returns
`IntegrityConfig` directly and `SealedConfig` parses it correctly.

The layout used a fixed formula `y = 40 + type_row * 140` where `type_row`
was the static position in `type_order`. When a type emitted more than 5 nodes
it added `sub_row` to `type_row`, causing its last sub-row to land at the same
y as the next type's first row.

## Goals / Non-Goals

**Goals:**
- Always show live cluster state, not stale local file.
- Collision-free node placement regardless of how many nodes each type has.
- Two-tier view (endpoint + backing module) matching standard architecture diagrams.
- Non-destructive filter that never mutates `this.nodes`/`this.edges`.
- Preserve manually-placed nodes across live reloads.

**Non-Goals:**
- Persisting filter state across page reloads.
- Animated layout transitions.
- Dependency-edge auto-generation from route relationships (edges still come from explicit `dependencies` map in routes).

## Decisions

### D1 — Two-pass layout via `TopologyLayout::build`
First pass: iterate all pending nodes (without positions), count per type.
Second pass: `TopologyLayout::build(type_order, counts)` computes
`base_row[type] = sum of rows used by preceding present types`.
`position(type, index) = (40 + col*300, 40 + (base_row + sub_row)*140)`.
This eliminates the fixed-offset assumption and works for any count.

### D2 — Two-tier node IDs: `route:{path}` and `wasm:{name}`
Endpoint nodes keep the existing `route:{path}` ID (backwards-compatible with
edge references). WASM backing nodes use `wasm:{name}` so they are distinct.
The connecting edge goes from `route:{path}` to `wasm:{name}`.

### D3 — Merge strategy in `loadLiveTopology`
`liveIds = Set(liveNodes.map(n => n.id))`
`manualNodes = this.nodes.filter(n => !liveIds.has(n.id))`
`this.nodes = [...liveNodes, ...manualNodes]`
Manual edges are kept only if both endpoints survive (avoids dangling pointers).
Live edges fully replace any previously live edges.

### D4 — Filter is a pure view transform
`computeFilteredGraph()` reads `this.nodes`/`this.edges` and returns a filtered
copy passed to `canvas.setGraph()`. Source arrays are never mutated. This means
node positions from drag are always preserved in `this.nodes` even when a node
is currently filtered out.

### D5 — Tags from `node.data.tags` (CSV)
No schema change needed. Operators add tags by editing a node in the topology
editor and setting `data.tags = "team-ai,feature-llm"`. `collectTags()` splits
on comma, trims, and deduplicates. Tag pills act as text-filter shortcuts.

## Risks / Trade-offs

- **Two-tier doubles node count**: a cluster with 20 user routes produces 40
  nodes (20 endpoint + 20 wasm). Layout remains readable up to ~25 nodes per
  type before horizontal overflow. Canvas panning handles the rest.
- **Manual nodes lost on hard refresh**: nodes added in edit mode but not backed
  by a real manifest route are in component state only. A page reload loses them.
  Accepted trade-off for now; localStorage persistence is a separate change.

## Open Questions

None.
