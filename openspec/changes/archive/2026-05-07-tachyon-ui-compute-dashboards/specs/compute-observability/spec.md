## ADDED Requirements

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
