# Design: full-navigation-implementation-and-wiew-isolation

## View model
The desktop shell keeps a single `<main>` container and swaps pre-rendered view sections in place. Each management plane receives a stable DOM identifier so routing logic does not depend on presentation copy:

- `view-dashboard`
- `view-topology`
- `view-registry`
- `view-identity`
- `view-account`
- `view-broker`

## Routing
- Sidebar links continue to use `data-view` attributes.
- `tachyon-ui/src/main.ts` owns the active-view state and GSAP enter/exit transitions.
- Opening Mesh Topology refreshes the graph through the existing `get_mesh_graph` Tauri command.

## Composition with IAM
This change does not own PAT issuance or security APIs. It only provides the dedicated account surface required by the IAM change.
