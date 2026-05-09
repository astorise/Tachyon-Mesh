# global-overview Specification

## Purpose
Define the authenticated default overview dashboard for Tachyon UI's web component shell.
## Requirements
### Requirement: Web component shell shows a global telemetry overview after login
The Tachyon web component shell SHALL provide a `<tachyon-overview-panel>` dashboard that extends `TachyonConfigDashboard` and is automatically mounted into `#router-view` after a successful `iam:authenticated` event.

#### Scenario: Overview is mounted after authentication
- **WHEN** the IAM layer emits `iam:authenticated`
- **THEN** `TachyonAppShell` displays the shell
- **AND** it mounts `<tachyon-overview-panel>` into the router view without requiring a sidebar click
- **AND** the overview navigation item is marked active

#### Scenario: Overview route is reachable from navigation
- **WHEN** the authenticated shell renders navigation links
- **THEN** it includes an Overview route
- **AND** selecting that route mounts `<tachyon-overview-panel>`

### Requirement: Global overview animates mesh telemetry counters
The `<tachyon-overview-panel>` dashboard SHALL render metric cards for active edge nodes, global Wasm instances, and AI/GPU utilization, and SHALL animate numeric counters from zero to their displayed values using GSAP.

#### Scenario: Counters animate on panel mount
- **WHEN** `<tachyon-overview-panel>` is connected
- **THEN** it renders a responsive grid of telemetry metric cards
- **AND** each numeric counter starts at zero
- **AND** GSAP animates each counter to its configured value

#### Scenario: Overview uses established visual styling
- **WHEN** the overview panel renders
- **THEN** it uses dark slate backgrounds, cyan highlights, and monospace numeric data
- **AND** it inherits shared dashboard styling through `TachyonConfigDashboard`

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

