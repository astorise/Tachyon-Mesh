## 1. Live manifest source

- [x] 1.1 Verify `get_admin_json::<SealedConfig>(ADMIN_MANIFEST_PATH)` correctly deserialises `IntegrityConfig` after the manifest-format fix
- [x] 1.2 Remove the `config_payload` wrapper deserialization from the topology path

## 2. tachyon:// URI and node classification

- [x] 2.1 Extend module-name extraction to check `targets[0].module` when top-level `module` field is absent
- [x] 2.2 Add `module_name.starts_with("tachyon://")` to `is_wasm_module` predicate alongside `.wasm` suffix

## 3. Two-tier node model

- [x] 3.1 Add `emit_endpoint_pair` flag for user routes with a WASM target
- [x] 3.2 Emit `endpoint` node (`route:{path}`) for user routes
- [x] 3.3 Emit `custom-wasm` node (`wasm:{name}`) for the backing module
- [x] 3.4 Emit edge from `route:{path}` → `wasm:{name}`
- [x] 3.5 Add `gRPC` protocol label for paths starting with `/grpc`

## 4. Two-pass layout engine

- [x] 4.1 Define `PendingNode` struct with `id`, `node_type`, `label`, `data`
- [x] 4.2 Refactor all `nodes.push(...)` + `topology_layout_position(...)` calls to `pending.push(...)` + `pending_type_counts` increment
- [x] 4.3 Implement `TopologyLayout::build` computing `base_rows` from per-type counts
- [x] 4.4 Implement `TopologyLayout::position` using `base_row + sub_row`
- [x] 4.5 Second pass: iterate `pending`, call `layout.position`, push `TopologyNodeSpec` into `nodes`
- [x] 4.6 Move `custom-wasm` to position 2 in `type_order` (was 4)

## 5. Frontend — merge on reload

- [x] 5.1 In `loadLiveTopology`: compute `liveIds`, keep `manualNodes` not in live set
- [x] 5.2 Merge `this.nodes = [...liveNodes, ...manualNodes]`
- [x] 5.3 Keep manual edges where both endpoints survive; replace live edges

## 6. Frontend — filter bar

- [x] 6.1 Add `filterText`, `filterTypes`, `showEdges` state to `TachyonTopologyPanel`
- [x] 6.2 Implement `computeFilteredGraph()` returning filtered nodes and edges
- [x] 6.3 Implement `collectTags()` from `node.data.tags` across all nodes
- [x] 6.4 Render filter bar: text input, type chips, tag pills, edge checkbox, counter, clear button
- [x] 6.5 Wire `input` event on text field → update `filterText`, call `pushGraphToCanvas`
- [x] 6.6 Wire type chip clicks → toggle `filterTypes`, call `refresh`
- [x] 6.7 Wire tag pill clicks → set `filterText` to tag, call `pushGraphToCanvas`
- [x] 6.8 Wire edge checkbox → update `showEdges`, call `pushGraphToCanvas`
- [x] 6.9 Wire clear button → reset all filter state, call `refresh`
- [x] 6.10 Update `pushGraphToCanvas` to pass `computeFilteredGraph()` result to canvas
- [x] 6.11 Implement `updateFilterBadge()` for live counter updates without full re-render

## 7. i18n

- [x] 7.1 Add `topology.filter.placeholder`, `.show-edges`, `.active`, `.clear` to English dictionary
- [x] 7.2 Add same keys to French dictionary
