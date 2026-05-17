# compute-observability Specification Delta

## ADDED Requirements

### Requirement: Gateway Routes Heavy BaaS Work To Ephemeral Compute
The system gateway SHALL route media range requests and analytical query payloads to dedicated ephemeral FaaS components instead of the normal OLTP path.

#### Scenario: Media range request is detected
- **WHEN** an incoming request has a `Range` header and targets a media path
- **THEN** the gateway forwards it to `system-faas-media-server`

#### Scenario: Analytical query is detected
- **WHEN** a query payload contains aggregation indicators such as `GROUP BY` or `SUM`
- **THEN** the gateway forwards it to `system-faas-olap-engine`

### Requirement: Zero-Copy Media Range WIT
The mesh SHALL expose a `tachyon:storage@1.1.0/media-stream` WIT interface for piping file byte ranges from RustFS to an output socket handle.

#### Scenario: Media server requests a byte range
- **WHEN** a media FaaS asks the host to pipe `start-byte..end-byte`
- **THEN** the host returns the number of bytes written or a string error

### Requirement: Media Server Returns Partial Content
The `system-faas-media-server` component SHALL parse HTTP byte range headers and format HTTP `206 Partial Content` responses with `Accept-Ranges`.

#### Scenario: Valid byte range arrives
- **WHEN** the component receives `Range: bytes=10-42`
- **THEN** it returns status `206`
- **AND** includes range response headers

### Requirement: OLAP Engine Aggregates In Isolation
The `system-faas-olap-engine` component SHALL execute bounded analytical aggregations inside its Wasm instance and return JSON results without loading the workload into core-host memory.

#### Scenario: Grouped rows are aggregated
- **WHEN** the OLAP engine receives rows with group and value fields
- **THEN** it returns a grouped sum result as JSON
