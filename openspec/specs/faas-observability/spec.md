## ADDED Requirements

### Requirement: FaaS SDK macro initializes lightweight JSON logging
The workspace SHALL provide a `faas-sdk` proc-macro crate exposing `#[faas_handler]`, and that macro SHALL inject a `tracing_subscriber` configured to emit JSON logs to `stdout` at the start of the annotated guest handler.

#### Scenario: Annotated guest handler enables structured logging
- **WHEN** a guest entrypoint is annotated with `#[faas_sdk::faas_handler]`
- **THEN** the generated handler initializes JSON-formatted tracing output to standard output before the handler body runs
- **AND** the resulting function remains invocable as the guest entrypoint under WASI

### Requirement: Guest functions can emit structured telemetry without direct network exporters
Guest functions SHALL be able to depend on `tracing`, use the `faas-sdk` macro on their entrypoint, and emit at least one structured log event that is written to `stdout` alongside the function execution.

#### Scenario: Guest logic emits a structured info event
- **WHEN** the instrumented guest function processes a request
- **THEN** the guest writes at least one JSON log line representing a `tracing::info!` event to `stdout`
- **AND** the guest still writes its final response payload to `stdout`

### Requirement: Host separates guest telemetry from guest response output
After guest execution, `core-host` SHALL read the captured `MemoryWritePipe`, parse line-delimited JSON log entries, forward recognized log entries into the host tracing pipeline, and return only non-log output as the HTTP response body.

#### Scenario: Host forwards guest logs and preserves the response body
- **WHEN** a guest writes JSON tracing events followed by a plain-text response to `stdout`
- **THEN** the host parses and forwards the JSON log lines through its own logging
- **AND** the host excludes those log lines from the returned HTTP body
- **AND** the host returns the remaining plain-text output as the function response
