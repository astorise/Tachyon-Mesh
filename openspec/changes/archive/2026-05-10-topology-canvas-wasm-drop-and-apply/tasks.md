# Tasks

## WASM File Drag-and-Drop

- [x] Add `dragenter` / `dragover` / `dragleave` / `drop` handlers to the canvas container in `TachyonTopologyCanvas`
- [x] Visual feedback on dragover (cyan border + glow, restored on dragleave/drop)
- [x] On `.wasm` file drop: dispatch `topology:wasm-dropped` with `{ name, path, x, y }`
- [x] `TachyonTopologyPanel` listens to `topology:wasm-dropped` and creates a `custom-wasm` node pre-filled with capability name, `^1.0.0` SemVer, and `assetSource` path
- [x] Drop hint text below canvas (`topology.drop.hint`)

## Apply Topology Button

- [x] Replace "Build Bundle" button with "Apply Topology" (i18n key `topology.apply-topology`)
- [x] Loading spinner state during apply (`applying` flag + re-render)
- [x] Serialise graph: filter `custom-wasm` nodes with `assetSource` → `BundleDependency[]`
- [x] `invoke("bundle_and_apply_manifest", { dependencies })` with real payload
- [x] Success path: show `topology.feedback.apply-success` with config version
- [x] Conflict path (428): dispatch `topology:conflict` window event + show error feedback
- [x] Error path: show error message in feedback zone
- [x] `TachyonAppShell` listens to `topology:conflict` and opens `TachyonBundleConflictModal`

## i18n

- [x] `topology.apply-topology` / `topology.applying` (EN + FR)
- [x] `topology.drop.hint` / `topology.drop.active` (EN + FR)
- [x] `topology.feedback.applying` / `apply-success` / `apply-conflict` / `wasm-dropped` (EN + FR)

## OpenSpec Corrections

- [x] Fix node type names: `custom_wasm` → `custom-wasm`, `kv_cache` → `kv-cache`
- [x] Acknowledge `TachyonNodeEditor` as already implemented
- [x] Align field name: `local_source` → `assetSource`
