## MODIFIED Requirements

### Requirement: System FaaS can read host telemetry snapshots through a privileged world
The workspace SHALL provide a `system-faas-guest` component world that imports
`tachyon:telemetry/reader` and `tachyon:mesh/scaling-metrics`, and `core-host`
SHALL only satisfy those imports for routes sealed as role `system`.

#### Scenario: System autoscaling metrics route exposes pending queue depth
- **WHEN** a request targets the sealed system route `/metrics/scaling`
- **AND** the corresponding guest component imports `tachyon:mesh/scaling-metrics`
- **THEN** `core-host` instantiates the component with the privileged linker
- **AND** the guest can read the pending queue size for `/api/guest-call-legacy`
- **AND** the guest returns Prometheus text containing that queue depth
