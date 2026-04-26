# Implementation Tasks

## Phase 1: Tauri Backend (`tachyon-ui/src-tauri`)
- [x] Implement `get_resources` Tauri command parsing the current workspace configuration.
- [x] Implement `save_resource` and `delete_resource` commands. Ensure they format the configuration correctly for the CLI/sealing engine.

## Phase 2: Frontend State & Views (`tachyon-ui`)
- [x] Create `src/stores/resourceStore.ts` with Zustand to interface with the new Tauri commands.
- [x] Build `ResourceBadge.tsx` for visual type indicators.
- [x] Build `ResourceEditorModal.tsx` handling the discriminated union form (Internal vs External fields).
- [x] Build `ResourceCatalogView.tsx` with a responsive grid/table, utilizing GSAP for row entrance animations.
- [x] Add the "Resource Catalog" link to the main navigation layout (Sidebar).

## Phase 3: MCP Server Update (`tachyon-mcp`)
- [x] Locate the tool registration block in `tachyon-mcp/src/main.rs`.
- [x] Implement the `list_mesh_resources` tool.
- [x] Implement the `register_mesh_resource` tool, ensuring it writes to the workspace config similarly to the Tauri backend.
- [x] Update MCP capabilities schema if necessary.

## Phase 4: Integration & Polish
- [x] Verify that a resource added via MCP immediately appears in the Tachyon Studio UI (may require file watcher or manual refresh in UI).
- [x] Ensure that editing an external resource requires a valid HTTPS URL (validation in both UI and MCP).