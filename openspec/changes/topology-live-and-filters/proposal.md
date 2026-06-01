## Why

The Topology Canvas was showing stale data from the local `integrity.lock` file
instead of the live node manifest, nodes were overlapping due to a fixed-row
layout that didn't account for overflow, and operators had no way to focus on a
subset of components. All three gaps together made the canvas impractical for
exploring a cluster with many deployed routes.

## What Changes

- **Live manifest source**: `get_topology_graph` now correctly parses `GET /admin/manifest` as `IntegrityConfig` directly (the prior implementation expected the on-disk wrapper format, silently falling back to the local file on parse failure).
- **Two-tier node model**: user routes backed by a WASM module emit two nodes — an `endpoint` (HTTP/gRPC entry point) and a `custom-wasm` (backing module) — connected by an edge, matching the architecture diagram style.
- **`tachyon://` URI detection**: asset URIs are recognised as WASM module references alongside `.wasm`-suffix names.
- **Two-pass layout engine**: `TopologyLayout::build` pre-computes base row indices from actual per-type counts so that a type's overflow sub-rows never collide with the next type's band.
- **`type_order` reordering**: `custom-wasm` moved from position 4 (y=600px, off-canvas for a 540px viewport) to position 2 (y=320px).
- **Merge-on-reload**: `loadLiveTopology` preserves manually-added nodes and edges that have no live counterpart, so user-drawn diagrams survive topology refreshes.
- **Filter bar**: text search (label + `data.tags`), type-chip multi-select, tag pills derived from `node.data.tags`, show/hide edges toggle, active-filter counter, and one-click clear.

## Capabilities

### New Capabilities

- `topology-live-and-filters`: Live topology canvas with two-tier node model, collision-free layout, and filter bar.

### Modified Capabilities

- `ui-wiring`: `TachyonTopologyPanel` and `TachyonTopologyCanvas` substantially updated.
- `tauri-configurator`: `get_topology_graph` Tauri command returns enriched node data (`nodeType` field, two-tier pairs).

## Impact

- **`tachyon-client/src/lib.rs`**: `get_topology_graph` rewritten — live source, `tachyon://` detection, two-tier pairs, `TopologyLayout` struct replacing `topology_layout_position`, route/kv-cache/resource nodes all use two-pass layout.
- **`tachyon-ui/src/components/domains/TachyonTopologyPanel.ts`**: filter state, `computeFilteredGraph`, `collectTags`, `pushGraphToCanvas` uses filtered graph, `loadLiveTopology` merge logic, filter UI, filter event handlers.
- **`tachyon-ui/src/utils/i18n.ts`**: four new keys (`topology.filter.*`) in both `en` and `fr`.
