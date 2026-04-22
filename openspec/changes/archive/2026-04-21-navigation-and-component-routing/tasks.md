# Tasks: Change 073 Implementation

- [x] Update `tachyon-ui/index.html` to wrap the dashboard in `view-dashboard`, add dedicated sidebar links, and create hidden `view-topology`, `view-deployments`, `view-identity`, and `view-ai-broker` containers.
- [x] Update `tachyon-ui/src/main.ts` to bind sidebar navigation, animate `switchView` with GSAP, update the active link classes, and log the navigation map to the console.
- [x] Add a stable `get_mesh_graph` command through `tachyon-ui/src/main.rs` and `tachyon-client/src/lib.rs`, then render topology, deployment, identity, and AI broker panels from the shared desktop workflows.
- [x] Validate the change with `openspec validate --changes`, `npm run build`, `cargo build -p tachyon-ui --release`, and view-level verification for the `Deployment Manager (Ready)` label.
