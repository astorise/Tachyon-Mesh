## ADDED Requirements

### Requirement: View / Edit mode toggle

`<tachyon-topology-panel>` SHALL expose a header toggle with two mutually exclusive states: **View** and **Edit**. The toggle MUST be rendered to the right of the panel title, on the same row as the existing zoom / compact / reset controls.

#### Scenario: Default mode on first mount is View

- **GIVEN** the operator opens the topology route for the first time in the current session
- **WHEN** `<tachyon-topology-panel>` mounts
- **THEN** the mode toggle reports `View` as selected
- **AND** the canvas renders the live graph (or the empty state) without any add-node form, "Apply Topology" button, or node-editor side panel visible

#### Scenario: Mode is persisted across reloads within the session

- **GIVEN** the operator switches the toggle to `Edit`
- **WHEN** the operator triggers a full page reload
- **THEN** the toggle is restored to `Edit` on remount
- **AND** the editing affordances are visible

#### Scenario: A new session defaults back to View

- **GIVEN** the operator switched to `Edit` and then closed the browser tab
- **WHEN** the operator opens a fresh tab and navigates to the topology route
- **THEN** the toggle reports `View` as selected
- **AND** `sessionStorage` is consulted exclusively (NOT `localStorage`)

### Requirement: Edit-mode-only affordances

In View mode, the following affordances SHALL be disabled or hidden:

- The "Add Node" form (`#add-node-form`) is hidden.
- The "Apply Topology" button (`#btn-apply-topology`) is hidden.
- Drag-to-move on nodes is disabled (pointer drag still pans the canvas).
- The wasm-drop target on the canvas is disabled (drop is rejected).
- The node-editor side panel does not open on node selection; clicking a node only highlights it.

#### Scenario: View mode hides edit controls

- **GIVEN** the panel is in View mode
- **WHEN** the panel renders
- **THEN** `#add-node-form` and `#btn-apply-topology` are not present in the DOM
- **AND** dragging a node has no visible effect on its `style.left` / `style.top`

#### Scenario: Edit mode restores every affordance

- **GIVEN** the operator switches to Edit mode
- **WHEN** the panel re-renders
- **THEN** `#add-node-form` and `#btn-apply-topology` are present
- **AND** dragging a node updates its position and dispatches `topology:node-moved`
- **AND** dropping a `.wasm` file dispatches `topology:wasm-dropped`

### Requirement: Mode is reflected in the source banner

The existing source banner (live / offline) SHALL append the current mode as a human-readable suffix.

#### Scenario: Live banner shows mode

- **GIVEN** `get_topology_graph` returned a live result and the panel is in `View` mode
- **WHEN** the panel renders the header
- **THEN** the banner text contains both the `topology.live-banner` content and a localised "View" suffix
- **AND** the suffix updates to "Edit" when the mode toggle is flipped

#### Scenario: Offline banner shows mode

- **GIVEN** `get_topology_graph` returned an empty graph and the panel is in `View` mode
- **WHEN** the panel renders the header
- **THEN** the banner text contains both the `topology.offline-banner` content and a localised "View" suffix

### Requirement: Topology empty state is covered by Playwright

The Playwright suite SHALL include a spec that asserts the empty-state card from `topology-canvas-taxonomy@2026-05-19` is rendered when `get_topology_graph` returns an empty graph, and that no `[data-node-id]` button exists in the canvas in that case.

#### Scenario: Empty backend produces empty canvas

- **GIVEN** the Tauri fake returns `{ nodes: [], edges: [], source: "live", status: "ok" }` from `get_topology_graph`
- **WHEN** the operator opens the topology route under Playwright
- **THEN** the empty-state card with the `topology.empty.title` label is visible
- **AND** zero elements matching `[data-node-id]` exist in the rendered shadow tree

### Requirement: Demo flag is covered by Playwright

The Playwright suite SHALL include a spec that asserts the `?demo=1` flag re-injects the sample graph and renders the "Demo data" banner.

#### Scenario: Demo flag restores sample graph

- **GIVEN** the Tauri fake returns an empty `get_topology_graph` response
- **AND** the application is loaded with `?demo=1` in the URL
- **WHEN** the operator opens the topology route under Playwright
- **THEN** at least one `[data-node-id]` element is present in the canvas
- **AND** a banner containing the `topology.demo.banner` text is rendered above the canvas
