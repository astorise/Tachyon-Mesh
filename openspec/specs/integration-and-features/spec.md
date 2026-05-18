# integration-and-features Specification

## Purpose
TBD - created by archiving change v1-1-audit-full-closure. Update Purpose after archive.
## Requirements
### Requirement: Truthful Audit Trail for Phase 3 Stubs
Archived changes whose code is unwired SHALL NOT carry `[x]` task marks.

#### Scenario: Phase 3 ghost specs are reset
- **GIVEN** archived change `2026-05-17-dynamic-geo-pinning`, `2026-05-17-cqrs-materialized-views`, or `2026-05-17-baas-advanced-capabilities`
- **WHEN** the tasks list is inspected
- **THEN** every task SHALL be `[ ]` until the underlying code is wired

### Requirement: Wasmtime Custom Metrics Linker Binding
The Wasmtime linker setup SHALL expose `push_custom_metric` to guest components, matching the WIT signature in `wit/telemetry/custom-metrics.wit`.

#### Scenario: Guest can invoke push_custom_metric
- **GIVEN** a guest component instantiated against the host Wasmtime linker
- **WHEN** the guest invokes the `push_custom_metric(name, value)` host import
- **THEN** the call SHALL succeed without `Linker` resolution errors
- **AND** the host SHALL record the metric in the custom metric registry

### Requirement: GitOps Canary Telemetry Evaluation
`systems/system-faas-gitops-broker` SHALL evaluate canary stage transitions against live telemetry rather than static mock responses.

#### Scenario: Canary advances when telemetry threshold is met
- **GIVEN** a canary deployment is in progress with a PromQL-style threshold definition
- **WHEN** the broker polls the metrics registry and the threshold is satisfied
- **THEN** the broker SHALL advance the canary to the next stage

#### Scenario: Canary holds when telemetry threshold is unmet
- **GIVEN** a canary deployment is in progress
- **WHEN** the broker polls the metrics registry and the threshold is NOT satisfied
- **THEN** the broker SHALL NOT advance the canary stage

### Requirement: Host-Guest Telemetry Integration Test
`core-host/tests/host_guest_integration_test.rs` SHALL execute a guest component that invokes the custom metrics host boundary and SHALL assert the host stored the metric.

#### Scenario: Integration test records guest metric
- **GIVEN** a minimal Wasmtime guest exercising the `push_custom_metric` import
- **WHEN** the integration test runs `cargo test --test host_guest_integration_test`
- **THEN** the test SHALL pass
- **AND** the host registry SHALL contain the metric emitted by the guest

