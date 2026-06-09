## MODIFIED Requirements

### Requirement: system-faas-metering aggregates usage events in the background
The host's background metering exporter SHALL own the in-memory aggregation of usage events: it accumulates `tachyon.telemetry.usage` records drained from the bounded telemetry queue and flushes a batch either when the batch reaches the configured size (`TELEMETRY_EXPORT_BATCH_SIZE`) or when the periodic flush interval elapses (`TELEMETRY_EXPORT_FLUSH_INTERVAL`, default 60 seconds), whichever comes first. The exporter SHALL forward each flushed batch to `system-faas-metering`, which SHALL operate strictly as the out-of-band persistence sink (appending the billing records to durable storage) and SHALL NOT back-pressure the request path.

#### Scenario: Batch flushes on size or interval
- **WHEN** the exporter has accumulated one or more usage records
- **THEN** it flushes once the batch reaches `TELEMETRY_EXPORT_BATCH_SIZE`
- **AND** it also flushes a non-empty batch when the flush interval (default 60 s) elapses
- **AND** flushing forwards the batch to `system-faas-metering` off the request path

#### Scenario: Downstream outage does not back-pressure requests
- **WHEN** a metering forward fails because the sink is temporarily unavailable
- **THEN** the exporter continues draining and flushing subsequent batches on each interval
- **AND** the host's request latency remains unaffected

## ADDED Requirements

### Requirement: Metering batches are durably staged before export
Before forwarding a metering batch, the host SHALL persist each record to the durable `metering_outbox` store. On a successful forward it SHALL delete those staged entries; on a failed forward the entries SHALL be retained so that no usage record is lost to a host crash or a downstream outage.

#### Scenario: Records survive a crash between staging and export
- **WHEN** the host persists a metering batch to the outbox and then crashes before the forward completes
- **THEN** the staged records remain in the `metering_outbox` table after restart

#### Scenario: Successful export clears the outbox; failure retains it
- **WHEN** a metering batch is forwarded successfully
- **THEN** the host deletes the corresponding `metering_outbox` entries
- **AND** a failed forward instead retains those entries durably for recovery
