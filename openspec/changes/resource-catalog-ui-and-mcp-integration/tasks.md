# Implementation Tasks

## Phase 1: Tauri Backend (`tachyon-ui/src-tauri`)
- [ ] Implement `get_resources` Tauri command parsing the current workspace configuration.
- [ ] Implement `save_resource` and `delete_resource` commands. Ensure they format the configuration correctly for the CLI/sealing engine.

## Phase 2: Frontend State & Views (`tachyon-ui`)
- [ ] Create `src/stores/resourceStore.ts` with Zustand to interface with the new Tauri commands.
- [ ] Build `ResourceBadge.tsx` for visual type indicators.
- [ ] Build `ResourceEditorModal.tsx` handling the discriminated union form (Internal vs External fields).
- [ ] Build `ResourceCatalogView.tsx` with a responsive grid/table, utilizing GSAP for row entrance animations.
- [ ] Add the "Resource Catalog" link to the main navigation layout (Sidebar).

## Phase 3: MCP Server Update (`tachyon-mcp`)
- [ ] Locate the tool registration block in `tachyon-mcp/src/main.rs`.
- [ ] Implement the `list_mesh_resources` tool.
- [ ] Implement the `register_mesh_resource` tool, ensuring it writes to the workspace config similarly to the Tauri backend.
- [ ] Update MCP capabilities schema if necessary.

## Phase 4: Integration & Polish
- [ ] Verify that a resource added via MCP immediately appears in the Tachyon Studio UI (may require file watcher or manual refresh in UI).
- [ ] Ensure that editing an external resource requires a valid HTTPS URL (validation in both UI and MCP).