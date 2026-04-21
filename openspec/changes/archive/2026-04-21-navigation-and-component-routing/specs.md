# Specifications: UI Routing

## 1. Frontend Router
In `main.ts`, implement a `switchView(viewName)` function. 
- It must hide the current content of the `<main>` container.
- It must inject the new template into the DOM or toggle visibility of pre-rendered `div` blocks.
- It must update the 'active' CSS class on the sidebar links.

## 2. Component Scopes
- **Mesh Topology:** Must call `tachyon_client::get_mesh_graph()`.
- **FaaS Registry:** Must include the upload button for Change 070.
- **Identity:** Must include the User table and MFA setup from Change 069.
- **AI Broker:** Must include the chunked upload bar from Change 071.