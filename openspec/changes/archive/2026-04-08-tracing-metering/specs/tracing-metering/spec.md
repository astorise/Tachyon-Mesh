## ADDED Requirements

### Requirement: Hosts can configure probabilistic telemetry sampling
The host manifest SHALL allow operators to configure a global telemetry sampling rate that determines whether a request incurs tracing and fuel-metering overhead.

#### Scenario: A request is sampled for telemetry
- **WHEN** an incoming request is selected by the configured sampling rate
- **THEN** the host enables request-specific metering and trace collection for that execution

#### Scenario: A request is not sampled for telemetry
- **WHEN** an incoming request is not selected by the configured sampling rate
- **THEN** the host executes the request without enabling trace generation or instruction counting overhead

### Requirement: Sampled telemetry is exported through a bounded asynchronous queue
The host SHALL enqueue completed sampled telemetry records into a bounded asynchronous channel without blocking request execution, and MAY drop new records when the queue is full.

#### Scenario: The telemetry queue accepts a sampled record
- **WHEN** a sampled request completes
- **AND** the telemetry queue has available capacity
- **THEN** the host formats the trace and metrics payload
- **AND** pushes it onto the queue without blocking the request path

#### Scenario: The telemetry queue is saturated
- **WHEN** a sampled request completes
- **AND** the telemetry queue is full
- **THEN** the host drops the telemetry payload instead of blocking or exhausting memory

### Requirement: Metering data is flushed by a background system FaaS
The host SHALL run a background exporter that consumes telemetry records from the queue and forwards them to a system FaaS without delaying primary request handling.

#### Scenario: A telemetry batch is exported
- **WHEN** the background exporter drains one or more telemetry records from the queue
- **THEN** it invokes the metering system FaaS with the batch payload
- **AND** the export path runs independently from primary request execution threads
