# Tasks

## Backend (tachyon-client)
- [x] Extend SealedConfig with `kv_caches: Vec<serde_json::Value>`
- [x] Add TopologyNodeSpec, TopologyEdgeSpec, TopologyGraphSpec structs
- [x] Implement get_topology_graph(): routes, kv_caches, resources → nodes
- [x] Infer node type from role, models[], module extension
- [x] Generate edges from route dependencies map
- [x] Auto-layout: group by type in horizontal bands, 5 nodes per sub-row
- [x] Graceful offline: return empty nodes + "offline" status when lockfile absent

## Tauri
- [x] Register `get_topology_graph` command in tachyon-ui/src/main.rs

## Canvas (TachyonTopologyCanvas)
- [x] Replace 960×540 fixed canvas with 1920×1080 virtual viewport
- [x] CSS transform (translate + scale) on viewport element
- [x] Wheel event: zoom toward cursor (0.2× – 4×)
- [x] Background pointer-drag: pan with setPointerCapture
- [x] zoomIn() / zoomOut() / resetView() public API
- [x] toggleCompact() / isCompact getter
- [x] Compact mode: 48×48 circle nodes with glyph + title tooltip
- [x] Edge SVG centers: 24px offset (bubble) vs card center (card)
- [x] Node drag: coordinates corrected for zoom level
- [x] setSelected(): lightweight ring update (no full re-render)

## Panel (TachyonTopologyPanel)
- [x] loadLiveTopology(): invoke("get_topology_graph") on connectedCallback
- [x] Replace sample nodes with live data when non-empty
- [x] Live-source banner (green) / offline banner (amber)
- [x] Toolbar: zoom in, zoom out, reset, compact toggle buttons
- [x] compact button label toggles between ⊞ and ☰

## i18n
- [x] 8 new EN + FR keys: zoom-in, zoom-out, zoom-reset, compact-mode,
      expand-mode, loading, offline-banner, live-banner
