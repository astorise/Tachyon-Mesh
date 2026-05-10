# Proposal: Interactive Topology Canvas — WASM File Drop and Visual Deployment

## Problem Statement

While the `<tachyon-topology-canvas>` and `<tachyon-node-editor>` deliver a
full read-write visual IDE (drag-and-drop repositioning, contextual type-specific
forms, node add/delete), two interaction layers were missing:

1. **OS file drag-and-drop** — operators cannot drag a local `.wasm` binary
   from their filesystem onto the canvas to instantly spawn a configured
   Custom WASM node. They must use the Add Node form and manually fill in
   the asset path.

2. **Visual deployment** — the "Build Bundle" button serialised the graph and
   emitted a `topology:serialize` event, but never actually called the
   backend pipeline. The `invoke("bundle_and_apply_manifest")` call in the
   AppShell shell was wired to an empty dependency list unrelated to the
   topology state.

## Objective

Close those two gaps:

1. **WASM File Drop**: drag `.wasm` files from the OS onto the canvas → a
   `custom-wasm` node is created at the drop coordinates, pre-filled with
   the file name as capability name, `^1.0.0` as the SemVer constraint, and
   the resolved local path as `assetSource`.

2. **Apply Topology**: the header button becomes a primary "Apply Topology"
   action that: serialises the graph, extracts `custom-wasm` nodes as bundle
   dependencies, invokes `bundle_and_apply_manifest` with real payload, shows
   a loading spinner, and routes conflict responses to the existing
   `TachyonBundleConflictModal`.

## Notes on the original spec

The original spec draft used `custom_wasm` (underscore) and `kv_cache`
(underscore) for node types. The canonical names in the codebase are
`custom-wasm` and `kv-cache` (kebab-case), matching the `TopologyNodeType`
union and all WIT interface identifiers. The spec is corrected to reflect this.

`TachyonNodeEditor` already existed as a full Shadow-DOM Web Component with
type-specific form fields for all eight node types; the spec is updated to
acknowledge this rather than requesting its creation.

The field name for the local binary path is `assetSource` (not `local_source`)
to match the existing editor and serialisation layer.
