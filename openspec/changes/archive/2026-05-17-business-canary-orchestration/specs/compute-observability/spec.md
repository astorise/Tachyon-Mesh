# compute-observability Specification Delta

## ADDED Requirements

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
