# compute-observability Specification

## Purpose
Expose workload orchestration and observability configuration dashboards in the Tachyon web component shell.
## Requirements
### Requirement: Web component shell exposes Workloads and Secrets configuration
The Tachyon web component shell SHALL provide a `<tachyon-workloads-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit execution engine and secret reference settings through the shared configuration command.

#### Scenario: Operator submits workload configuration
- **WHEN** the operator selects an execution engine and enters a Vault secret reference
- **THEN** the panel invokes `apply_configuration` with the workloads domain
- **AND** the payload includes the selected engine and secret reference
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Workloads panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Workloads route
- **AND** selecting that route mounts `<tachyon-workloads-panel>`

### Requirement: Web component shell exposes Observability configuration
The Tachyon web component shell SHALL provide a `<tachyon-observability-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit OTLP endpoint and log level settings through the shared configuration command.

#### Scenario: Operator submits observability configuration
- **WHEN** the operator enters an OTLP endpoint URL and selects a log level
- **THEN** the panel invokes `apply_configuration` with the observability domain
- **AND** the payload includes the endpoint and log level values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Observability panel allows telemetry export to be disabled
- **WHEN** the operator leaves the OTLP endpoint empty and submits a log level
- **THEN** the panel invokes `apply_configuration` with a null or empty endpoint value
- **AND** the backend accepts the configuration as local logging without trace export

#### Scenario: Observability panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes an Observability route
- **AND** selecting that route mounts `<tachyon-observability-panel>`

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

### Requirement: Per-User IAM Audit Logs
The core host SHALL expose `GET /admin/logs` returning the IAM audit
ring buffer in newest-first order, with optional `user` and `lines`
query parameters. The `user` filter SHALL match entries whose
`target_user` or `actor` equals the supplied value. The `lines` filter
SHALL clamp between 1 and 500, defaulting to 50 when omitted.

#### Scenario: User filter returns matching entries
- **GIVEN** the audit buffer contains entries for several users
- **WHEN** an admin requests `GET /admin/logs?user=alice`
- **THEN** the response contains only entries whose `actor` or
  `target_user` is `alice`
- **AND** the response is ordered newest-first

#### Scenario: Lines parameter clamps to 500
- **WHEN** an admin requests `GET /admin/logs?lines=10000`
- **THEN** the response contains at most 500 entries
- **AND** no error is returned

#### Scenario: Default response without filters
- **WHEN** an admin requests `GET /admin/logs` with no parameters
- **THEN** the response contains the most recent 50 audit entries
- **AND** the entries are not filtered by user

### Requirement: Custom Telemetry Metrics WIT
The mesh SHALL expose a `tachyon:telemetry@1.1.0/custom-metrics` WIT interface that lets trusted Wasm bridge components submit counter, gauge, and histogram metric samples with string labels.

#### Scenario: Bridge component submits a custom metric
- **WHEN** a bridge component calls `custom-metrics.push` with a metric name, value, metric type, and labels
- **THEN** the host accepts the sample or returns a human-readable validation error
- **AND** the WIT contract remains versioned under `tachyon:telemetry@1.1.0`

### Requirement: Core Host Translates Custom Metrics To Prometheus
The core host SHALL dynamically create and cache Prometheus collectors for custom metrics and register them in the default Prometheus registry.

#### Scenario: Gauge metric is pushed repeatedly
- **WHEN** two gauge samples with the same metric name and label keys are pushed
- **THEN** the first call creates and registers the collector
- **AND** the second call reuses the cached collector and updates the latest value

### Requirement: Canary Evaluation Supports Business Metrics
The canary evaluator SHALL evaluate custom metric thresholds declared on a route before stepping traffic forward.

#### Scenario: Custom metric violates threshold
- **GIVEN** a route canary declares a custom metric threshold
- **WHEN** the latest observed metric value violates that threshold
- **THEN** the evaluator sets the canary traffic weight to zero
- **AND** the rollout phase records a rollback reason naming the metric

### Requirement: Manifest Validation Accepts Canary Metric Thresholds
The configuration API SHALL accept `canary.metrics` entries containing `name` and `threshold` fields, and SHALL reject malformed custom metric thresholds during dry-run validation.

#### Scenario: Canary metrics are declared in a manifest
- **WHEN** a manifest route contains `canary.metrics` with non-empty names and comparison thresholds
- **THEN** validation succeeds
- **AND** malformed threshold strings are reported as validation errors

### Requirement: Gateway Routes Heavy BaaS Work To Ephemeral Compute
The system gateway SHALL route media range requests and analytical query payloads to dedicated ephemeral FaaS components instead of the normal OLTP path.

#### Scenario: Media range request is detected
- **WHEN** an incoming request has a `Range` header and targets a media path
- **THEN** the gateway forwards it to `system-faas-media-server`

#### Scenario: Analytical query is detected
- **WHEN** a query payload contains aggregation indicators such as `GROUP BY` or `SUM`
- **THEN** the gateway forwards it to `system-faas-olap-engine`

### Requirement: Zero-Copy Media Range WIT
The mesh SHALL expose a `tachyon:storage@1.1.0/media-stream` WIT interface for piping file byte ranges from RustFS to an output socket handle.

#### Scenario: Media server requests a byte range
- **WHEN** a media FaaS asks the host to pipe `start-byte..end-byte`
- **THEN** the host returns the number of bytes written or a string error

### Requirement: Media Server Returns Partial Content
The `system-faas-media-server` component SHALL parse HTTP byte range headers and format HTTP `206 Partial Content` responses with `Accept-Ranges`.

#### Scenario: Valid byte range arrives
- **WHEN** the component receives `Range: bytes=10-42`
- **THEN** it returns status `206`
- **AND** includes range response headers

### Requirement: OLAP Engine Aggregates In Isolation
The `system-faas-olap-engine` component SHALL execute bounded analytical aggregations inside its Wasm instance and return JSON results without loading the workload into core-host memory.

#### Scenario: Grouped rows are aggregated
- **WHEN** the OLAP engine receives rows with group and value fields
- **THEN** it returns a grouped sum result as JSON

