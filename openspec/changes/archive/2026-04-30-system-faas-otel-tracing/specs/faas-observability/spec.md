## ADDED Requirements

### Requirement: OTLP tracing export is delegated to a System FaaS
The workspace SHALL provide a `system-faas-otel` component that accepts sampled Tachyon telemetry batches and converts them into trace span records without adding OpenTelemetry exporter dependencies to `core-host`.

#### Scenario: Telemetry batch is normalized into span records
- **GIVEN** a POST request containing host telemetry JSON or NDJSON
- **WHEN** `system-faas-otel` handles the request
- **THEN** it emits normalized span records containing the trace id, route name, status, and Tachyon duration attributes

### Requirement: OTLP export remains optional
`system-faas-otel` SHALL forward normalized span records to a sealed outbound endpoint when one is provided and SHALL write them to a sealed local volume when no endpoint is configured.

#### Scenario: Endpoint header is configured
- **GIVEN** a telemetry POST includes `x-tachyon-otlp-endpoint`
- **WHEN** `system-faas-otel` handles the batch
- **THEN** it forwards the normalized span payload through the `outbound-http` host import

#### Scenario: Endpoint header is omitted
- **GIVEN** a telemetry POST omits `x-tachyon-otlp-endpoint`
- **WHEN** `system-faas-otel` handles the batch
- **THEN** it appends the normalized span payload to `/app/data/otel-spans.ndjson`
