# compute-observability

## ADDED Requirements

### Requirement: Observability Panel Surfaces Runtime Telemetry
The `<tachyon-observability-panel>` component SHALL render runtime metrics,
recent log lines, and shadow divergences alongside the OTLP configuration
form, sourced from the `get_metrics`, `tail_logs`, and `get_shadow_diffs`
Tauri commands respectively.

#### Scenario: Live metrics, logs, and shadow diffs are visible
- **WHEN** `<tachyon-observability-panel>` finishes loading
- **THEN** it shows error rate, p50 and p99 latency, queue depth, and the
  metrics source from the latest `get_metrics` response
- **AND** it shows up to 50 most recent log lines from `tail_logs`
- **AND** it shows shadow divergences from `get_shadow_diffs` with the
  primary and shadow status codes when present

#### Scenario: Manual refresh updates all three sections
- **GIVEN** the panel is open
- **WHEN** the operator clicks the refresh control
- **THEN** the metrics, logs, and shadow sections are re-fetched in a
  single pass
- **AND** sections that fail to load degrade gracefully to empty-state
  copy without breaking the form below

### Requirement: Routing and Storage Show Sealed State
The `<tachyon-routing-panel>` component SHALL display a read-only preview
of the sealed routes above the configuration form, sourced from
`get_mesh_graph`. The `<tachyon-storage-panel>` component SHALL display a
read-only preview of workspace overlay resources above the configuration
form, sourced from `get_resources`.

#### Scenario: Routing panel previews sealed routes
- **WHEN** `<tachyon-routing-panel>` finishes loading
- **THEN** it lists the sealed routes (name, path, target count, TEE
  flag) above the configuration form
- **AND** it shows an empty-state message when no sealed routes exist

#### Scenario: Storage panel previews overlay resources
- **WHEN** `<tachyon-storage-panel>` finishes loading
- **THEN** it lists workspace overlay resources (name, type, target,
  pending flag) above the configuration form
- **AND** it shows an empty-state message when no overlay resources
  exist
