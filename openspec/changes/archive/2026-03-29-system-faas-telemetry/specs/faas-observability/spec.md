## ADDED Requirements

### Requirement: System FaaS can read host telemetry snapshots through a privileged world
The workspace SHALL provide a `system-faas-guest` component world that imports `tachyon:telemetry/reader`, and `core-host` SHALL only satisfy that import for routes sealed as role `system`.

#### Scenario: System route exposes telemetry metrics successfully
- **WHEN** a request targets a sealed route whose role is `system`
- **AND** the corresponding guest component imports `tachyon:telemetry/reader`
- **THEN** `core-host` instantiates the component with the privileged linker
- **AND** the guest can read host telemetry counters and return a metrics response

#### Scenario: User route cannot instantiate the privileged telemetry guest
- **WHEN** the same telemetry guest component is executed through a sealed route whose role is `user`
- **THEN** `core-host` does not provide the privileged telemetry import
- **AND** component instantiation fails before guest code runs

### Requirement: Host sheds privileged telemetry routes under heavy business load
`core-host` SHALL track active requests and reject sealed `system` routes once active load passes the configured threshold, so normal guest traffic keeps priority over telemetry exports.

#### Scenario: System metrics route is shed under pressure
- **WHEN** the active request count is above the system route load-shedding threshold
- **AND** an incoming request targets a sealed route whose role is `system`
- **THEN** `core-host` returns `503 Service Unavailable`
- **AND** it skips guest execution for that system route
