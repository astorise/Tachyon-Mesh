# Design: Resource Catalog & MCP API

## 1. Tachyon UI Architecture (`tachyon-ui`)
The UI will rely on a new Zustand store to manage the state of the workspace's resources.

### Store: `useResourceStore.ts`
- Manages an array of `MeshResource` objects (Name, Type, Target, Config).
- Handles Tauri IPC calls to read/write the workspace configuration.

### Views & Components
- `ResourceCatalogView.tsx`: A data table or grid displaying all resources. Includes filtering (Internal vs External) and search.
- `ResourceBadge.tsx`: A small visual indicator (e.g., Green for Internal/IPC, Blue for External/Egress).
- `ResourceEditorModal.tsx`: A form to create/edit a resource. 
  - Dynamic fields: If `type === 'external'`, show `allowed_methods`. If `type === 'internal'`, show `version_constraint`.

## 2. Tachyon MCP Architecture (`tachyon-mcp`)
The Rust-based MCP server needs to expose new tools to connected LLM clients.

### MCP Tools to Expose:
- `list_mesh_resources`: Returns the current map of logical resources from the project configuration.
  - *Use Case:* An AI checking if `stripe-api` is already configured before writing a Wasm payment module.
- `register_mesh_resource`: Accepts JSON matching the resource schema (name, type, target) and adds it to the workspace configuration.
  - *Use Case:* An AI generating a GitHub integration Wasm module can automatically request the addition of `https://api.github.com` as an external resource.

## 3. Tauri Backend API (`tachyon-ui/src-tauri`)
- Add commands: `get_resources`, `save_resource`, `delete_resource`. 
- Modifying a resource via these commands must automatically invoke the underlying logic to update the configuration payload and request an `integrity.lock` re-seal.