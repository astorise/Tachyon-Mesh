# faas-observability Delta

## ADDED Requirements

### Requirement: Telemetry levels MUST be hot-reloadable
The runtime SHALL adjust log levels and distributed tracing sampling rates dynamically based on the declarative GitOps state without restarting the host daemon.

#### Scenario: Enabling debug logs for a specific component
- **WHEN** the config API updates the `ObservabilityAndCompute` state with a debug override for `system-faas-authz`
- **THEN** the logging subsystem immediately begins emitting DEBUG level logs only for the `system-faas-authz` target, leaving the rest of the mesh at INFO level.
