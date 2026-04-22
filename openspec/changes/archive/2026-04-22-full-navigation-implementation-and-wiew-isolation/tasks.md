# Tasks: full-navigation-implementation-and-wiew-isolation

- [x] Normalize the change artifacts to the current OpenSpec delta layout for the desktop UI capability.
- [x] Rename the isolated desktop view IDs and sidebar routes so Registry, Broker, Identity, and Account surfaces are explicit and non-overlapping.
- [x] Keep `get_mesh_graph` wired through the Tauri backend and use it from the topology view without a full-page reload.
- [x] Validate the navigation flow with the existing GSAP SPA transitions, `npm run build`, and `openspec validate --changes`.
