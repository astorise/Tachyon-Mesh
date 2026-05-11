# Proposal: Topology Canvas — Live Data, Zoom/Pan, and Compact Bubble Mode

## Problems

1. **Mocked data** — The topology canvas displayed a static set of hardcoded
   `DEFAULT_NODES/DEFAULT_EDGES`. As the mesh grows, the canvas showed the same
   8 sample nodes regardless of what was actually deployed, making it decorative
   rather than operational.

2. **Scalability** — Full-card nodes (256×80 px) become unreadable as the number
   of deployed routes, kv-caches, and resources increases. With 20+ components,
   the canvas is too dense to navigate.

3. **No navigation** — No zoom, no pan. The canvas was a fixed 960×540 viewport
   with no way to explore areas outside the initial view.

## What Changes

### Backend — `tachyon-client`

New `get_topology_graph()` function reads `integrity.lock` (or the connected
node's live config) and reconstructs the full component graph:

- **Routes** → inferred as `endpoint`, `system-faas`, `llm`, or `custom-wasm`
  based on `role`, presence of `models[]`, and `module` file extension.
- **kv_caches** entries → `kv-cache` nodes linked to their bound LLM.
- **resources** → `external-resource` nodes.
- **Edges** from route `dependencies` maps (route A depends on route B → edge).
- **Auto-layout** groups nodes by type in horizontal bands (endpoints top,
  system-faas below, llm/kv-cache middle, storage/messaging at bottom), each
  band wrapping to sub-rows after 5 nodes per row.
- Returns `offline` status + empty nodes when `integrity.lock` is absent,
  so the panel gracefully falls back to the sample topology.

New public types: `TopologyNodeSpec`, `TopologyEdgeSpec`, `TopologyGraphSpec`.

### Tauri command

`get_topology_graph()` registered as a Tauri command in `tachyon-ui/src/main.rs`.

### Canvas — `TachyonTopologyCanvas`

**Zoom / pan:**
- A virtual 1920×1080 viewport (`canvas-viewport`) is placed inside a clipping
  container (`canvas-outer`). CSS `transform: translate(x, y) scale(z)` is
  applied to the viewport.
- Mouse wheel → zoom toward cursor (zoom range: 0.2× – 4×).
- Pointer-drag on canvas background → pan. `setPointerCapture` on the outer
  container prevents interference with node drag events.
- Toolbar buttons: zoom in (+0.2), zoom out (−0.2), reset (1×, centered).

**Compact bubble mode:**
- Toggle button (⊞ / ☰) switches between card view and bubble view.
- In bubble mode, each node is a 48×48 rounded circle showing only the type
  glyph. The full label appears as a native `title` tooltip on hover.
- SVG edges anchor to the center of circles (24px offset) in bubble mode
  vs. the card center in card mode.
- Mode switch triggers a full re-render; zoom/pan state is preserved.

**Node drag** is corrected to account for the current zoom level: pointer
coordinates are divided by `this.zoom` before clamping to virtual canvas bounds.

### Panel — `TachyonTopologyPanel`

- `connectedCallback` calls `invoke("get_topology_graph")` after rendering.
  If the call returns nodes, they replace the sample set and the canvas refreshes.
  If the call fails (offline / Tauri unavailable), the sample topology is kept
  silently.
- A live-source banner shows "Live topology from {url}" in green when real data
  is loaded, or "Offline — showing sample topology" in amber otherwise.
- Toolbar with four icon buttons above the canvas: +, −, ⊙ (reset), ⊞/☰.

### i18n

8 new keys: `topology.zoom-in`, `topology.zoom-out`, `topology.zoom-reset`,
`topology.compact-mode`, `topology.expand-mode`, `topology.loading`,
`topology.offline-banner`, `topology.live-banner` (EN + FR).
