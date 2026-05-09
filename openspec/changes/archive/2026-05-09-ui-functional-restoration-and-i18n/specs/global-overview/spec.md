# global-overview

## ADDED Requirements

### Requirement: Overview Consumes Live Runtime Metrics
The `<tachyon-overview-panel>` component SHALL combine
`MeshGraphSnapshot` with the live `RuntimeMetrics` exposed via the
`get_metrics` Tauri command when the host is reachable.

#### Scenario: Live metrics drive the visible counters
- **GIVEN** the host responds to `get_metrics`
- **WHEN** `<tachyon-overview-panel>` finishes loading
- **THEN** the "Global Wasm Instances" card reflects
  `RuntimeMetrics.queueDepth`
- **AND** the "AI/GPU Utilization" card is derived from
  `RuntimeMetrics.errorRate` and `RuntimeMetrics.p99LatencyMs`
- **AND** the status badge displays `RuntimeMetrics.source`

#### Scenario: Sealed-config fallback when host is offline
- **GIVEN** the host does not respond to `get_metrics`
- **WHEN** `<tachyon-overview-panel>` finishes loading
- **THEN** the visible metrics fall back to values derived from the
  sealed `MeshGraphSnapshot`
- **AND** the status badge reflects the sealed snapshot status

### Requirement: Component Registry Has No Topology Placeholder
The component registry SHALL NOT expose a `topology` route until a
dedicated topology component is shipped, and the registry SHALL deduplicate
the asset registry surface to a single `supply-chain` route.

#### Scenario: Sidebar excludes phantom routes
- **WHEN** the shell renders the sidebar from `listComponentRoutes()`
- **THEN** the rendered routes do not include `topology`
- **AND** they do not include a duplicate `registry` entry pointing to the
  supply-chain panel
