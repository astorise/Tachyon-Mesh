# system-faas-prom Specification

## Purpose
TBD - created by archiving change spec-coverage-closure. Update Purpose after archive.
## Requirements
### Requirement: system-faas-prom exposes host telemetry as a Prometheus exposition
The `system-faas-prom` WASM component SHALL read the host telemetry snapshot through the privileged `tachyon:mesh/telemetry-reader` world (`get-metrics`) and render it as a Prometheus text-format exposition. It SHALL respond to any inbound `handle-request` invocation with HTTP `200` and the exposition as the response body. The component itself performs no authentication and no HTTP-method filtering; restricting who may scrape the endpoint is the host's routing responsibility.

#### Scenario: Metrics are rendered on request
- **WHEN** the component receives any `handle-request` invocation
- **THEN** it calls `telemetry_reader::get-metrics` exactly once
- **AND** returns status `200` with a Prometheus text-format body

### Requirement: Exposition covers the full host request-telemetry metric set
The exposition SHALL emit exactly the following nine series, each immediately preceded by its `# TYPE` line and sourced from the corresponding telemetry-snapshot field:

- `tachyon_requests_total` (counter) — total requests received
- `tachyon_requests_completed_total` (counter) — requests completed
- `tachyon_requests_error_total` (counter) — requests that errored
- `tachyon_active_requests` (gauge) — requests currently in flight
- `tachyon_telemetry_dropped_events_total` (counter) — telemetry events dropped
- `tachyon_last_status` (gauge) — last observed response status code
- `tachyon_total_duration_us_total` (counter) — cumulative end-to-end duration (µs)
- `tachyon_total_wasm_duration_us_total` (counter) — cumulative guest execution duration (µs)
- `tachyon_total_host_overhead_us_total` (counter) — cumulative host overhead (µs)

#### Scenario: Every series carries a TYPE annotation
- **WHEN** a scraper reads the exposition body
- **THEN** each of the nine `tachyon_*` series is preceded by a `# TYPE <name> <counter|gauge>` line
- **AND** each rendered value is taken from the matching telemetry-snapshot field

#### Scenario: Exposition is valid with no traffic
- **WHEN** the host has served no requests and the component is scraped
- **THEN** every counter and gauge renders as `0`
- **AND** the response is still a valid `200` Prometheus exposition

