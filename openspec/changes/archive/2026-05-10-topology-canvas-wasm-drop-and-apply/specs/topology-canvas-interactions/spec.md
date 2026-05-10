# Interactive Topology Canvas Specifications

## 1. WASM File Drop Support

`<tachyon-topology-canvas>` handles OS-level HTML5 drag-and-drop of `.wasm`
files onto the canvas surface.

**Logic Requirements:**
- Prevent default browser behavior on `dragenter` and `dragover`.
- Apply a visual indicator (cyan border glow) while files are dragged over the
  canvas; remove it on `dragleave` and `drop`.
- On `drop`, read the `dataTransfer.files` array. Ignore non-`.wasm` entries.
- For each `.wasm` file:
  - Extract the file name (e.g., `ai-filter.wasm`), strip the `.wasm`
    extension, and use it as the initial capability name.
  - Resolve the local path from `file.path` if available (Tauri exposes this
    for native drop events), otherwise fall back to `file.name` as a
    placeholder that the operator can refine in the node editor.
  - Dispatch a `topology:wasm-dropped` custom event (bubbles + composed) with
    `{ name, path, x, y }` where `x` / `y` are the drop coordinates clamped
    to the canvas bounds.
- `TachyonTopologyPanel` listens to `topology:wasm-dropped` and creates a new
  node with `type: "custom-wasm"` (kebab-case), `semver: "^1.0.0"`, and
  `assetSource: path` at the event coordinates.

> **Note:** Node types use kebab-case identifiers: `custom-wasm`, `kv-cache`,
> `system-faas`, etc. — not underscore variants.

## 2. Contextual Node Editor (`<tachyon-node-editor>`)

`<tachyon-node-editor>` is an existing Shadow-DOM Web Component that renders
as a fixed right-aligned sliding sidebar (`fixed right-0 top-0 h-screen w-96`).

**Dynamic Form Fields per Node Type:**
- **`llm`:** Model name (text), Quantization (INT4 / INT8 / FP16 select),
  LoRA mode (dynamic / static select).
- **`kv-cache`:** Capacity GB (number), Eviction policy (LRU / FIFO select).
- **`custom-wasm`:** Capability name (text), SemVer constraint (text, monospace),
  Local asset path / `assetSource` (text, monospace, editable).
- **`endpoint`:** Protocol (HTTP / HTTPS / TCP / UDP select), Port (number).
- **`storage`:** Mount path (text).
- **`message-broker`:** Queue name (text).
- **`external-resource`:** Target URL, Authentication type, Timeout (ms).
- **`system-faas`:** Component name (text).

Changes committed via the "Save Node" button emit `topology:node-updated`,
which the panel uses to update its internal state and refresh the canvas badge.

## 3. Graph Serialization and the "Apply Topology" Button

`TachyonTopologyPanel` replaces the previous "Build Bundle" button with a
primary **"Apply Topology"** action.

**Serialization Logic:**
When clicked, the panel:
1. Iterates over nodes with `type === "custom-wasm"` that have a non-empty
   `assetSource`.
2. Maps each to a `BundleDependency`:
   - `name`: `data.capabilityName || label || id`
   - `version`: `data.semver || "^1.0.0"`
   - `source`: `data.assetSource`
3. Calls `invoke("bundle_and_apply_manifest", { dependencies })`.

**UX During Deployment:**
- Button is disabled and displays a CSS spinner + "Applying…" label.
- On success (HTTP 200): displays `topology.feedback.apply-success` with the
  new config version number.
- On conflict (HTTP 428 / `requiresResolution: true`): dispatches
  `topology:conflict` on `window`; `TachyonAppShell` intercepts and opens
  `<tachyon-bundle-conflict-modal>` with the conflict list.
- On any other error: displays the error message in the feedback zone.
