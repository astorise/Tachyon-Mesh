## MODIFIED Requirements

### Requirement: MCP Runtime Metrics
The MCP server SHALL expose a `tachyon_get_metrics` tool that returns active node telemetry through tachyon-client bindings, including scope denial totals introduced by faas-wit-import-scoping.

#### Scenario: Agent queries telemetry
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_get_metrics`
- **WHEN** an active Tachyon node connection is configured
- **THEN** tachyon-client queries the admin metrics endpoint
- **AND** the MCP response includes error rate, p50 latency, p99 latency, queue depth, and `scope_denial_total`
- **AND** `scope_denial_total` is the lifetime count of runtime WIT import denials across all deployments and categories
