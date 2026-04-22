# Specifications: Navigation & Component Routing

## 1. View Architecture (DOM)
The `<main>` container must be partitioned into the following ID-based views:
- `#view-dashboard`: Main telemetry metrics.
- `#view-topology`: Visual graph of P2P peers (Placeholder for now).
- `#view-registry`: The WASM Asset Registry upload forms.
- `#view-broker`: The Large Model (LLM) streaming interface.
- `#view-identity`: (Future) User and Role management.

## 2. Navigation Controller (`main.ts`)
- Implement a function `MapsTo(viewId: string)`.
- **Transition:** Use GSAP to fade out the current view and slide in the new one.
- **State:** Update the sidebar link styles (active/inactive) to reflect the current position.

## 3. Missing Backend Integration
- **Command:** Add `#[tauri::command] async fn get_mesh_graph() -> Result<String, String>` to `main.rs`.
- **Wiring:** This command must call `tachyon_client::get_mesh_graph()` (which you must ensure exists or mock it in the client).