## ADDED Requirements

### Requirement: Honest empty state when the backend reports no nodes

`<tachyon-topology-panel>` SHALL NOT render the previous `DEFAULT_NODES` / `DEFAULT_EDGES` sample graph as a fallback when `get_topology_graph` returns an empty node list. Instead, the panel SHALL render an empty-state card that clearly states no topology data was received and that links the operator to the new `nodes` view.

#### Scenario: Empty backend response shows empty state, not sample data

- **GIVEN** `get_topology_graph` returns `{ nodes: [], edges: [], source: "live", status: "ok" }`
- **WHEN** `<tachyon-topology-panel>` mounts
- **THEN** the canvas area renders an empty-state card with the message "No topology data received from the mesh"
- **AND** the card contains a link or button that navigates to the `nodes` route
- **AND** no `<button data-node-id>` element exists in the rendered canvas

#### Scenario: Demo flag re-enables sample graph for development only

- **GIVEN** the application is loaded with `?demo=1` in the URL
- **WHEN** `<tachyon-topology-panel>` mounts and `get_topology_graph` returns an empty node list
- **THEN** the panel loads the sample graph from `topology.demo.ts` and renders it as before
- **AND** an unmistakable banner reading "Demo data — not connected to a real mesh" is rendered above the canvas

### Requirement: Live vs offline source banner remains accurate

The existing live / offline banner SHALL reflect whether `get_topology_graph` returned non-empty data, regardless of whether the panel is currently displaying the demo fallback under `?demo=1`.

#### Scenario: Offline banner is shown for empty response without demo flag

- **GIVEN** `get_topology_graph` returns an empty node list and `?demo=1` is not set
- **WHEN** the panel renders
- **THEN** the header displays the offline banner styled with `text-amber-400/70`
- **AND** the canvas area shows the empty-state card defined above

#### Scenario: Demo banner overrides live banner under ?demo=1

- **GIVEN** `?demo=1` is set and the panel falls back to the sample graph
- **WHEN** the panel renders
- **THEN** the header MUST NOT display a `topology.live-banner` text claiming the data is live
- **AND** the demo banner from the previous requirement is the only banner shown
