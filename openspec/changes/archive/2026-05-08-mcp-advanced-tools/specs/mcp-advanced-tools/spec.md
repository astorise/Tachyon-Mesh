## ADDED Requirements

### Requirement: MCP Manifest Dry Run
The MCP server SHALL expose a `tachyon_dryrun_manifest` tool that validates a manifest payload without writing the workspace overlay, replacing `integrity.lock`, or applying state to a Tachyon node.

#### Scenario: Agent validates a manifest before sealing
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_dryrun_manifest`
- **AND** the arguments contain a manifest or sealed manifest `configPayload`
- **WHEN** the tool is executed
- **THEN** the server returns a structured validation report
- **AND** no local or remote Tachyon configuration state is persisted

### Requirement: MCP Runtime Metrics
The MCP server SHALL expose a `tachyon_get_metrics` tool that returns active node telemetry through tachyon-client bindings.

#### Scenario: Agent queries telemetry
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_get_metrics`
- **WHEN** an active Tachyon node connection is configured
- **THEN** tachyon-client queries the admin metrics endpoint
- **AND** the MCP response includes error rate, p50 latency, p99 latency, and queue depth data

### Requirement: MCP Log Notifications
The MCP server SHALL expose a `tachyon_tail_logs` tool that returns recent logs and notification-compatible `notifications/message` JSON-RPC payloads.

#### Scenario: Agent requests log tailing
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_tail_logs`
- **WHEN** recent logs are available from the active Tachyon node
- **THEN** the MCP response includes the log entries
- **AND** the response exposes matching `notifications/message` payloads for MCP clients that consume notifications

### Requirement: MCP Shadow Diff Analysis
The MCP server SHALL expose a `tachyon_get_shadow_diffs` tool that retrieves divergence data from the Tachyon shadow proxy.

#### Scenario: Agent reviews shadow traffic divergence
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_get_shadow_diffs`
- **WHEN** an active Tachyon node connection is configured
- **THEN** tachyon-client queries the shadow diff admin endpoint
- **AND** the MCP response contains the latest divergence records

### Requirement: MCP Chaos Scenario Execution
The MCP server SHALL expose a `tachyon_run_chaos_scenario` tool that starts a predefined chaos harness scenario through tachyon-client.

#### Scenario: Agent starts a chaos scenario
- **GIVEN** an MCP client calls `tools/call` with `name` set to `tachyon_run_chaos_scenario`
- **AND** the arguments contain a supported scenario name
- **WHEN** tachyon-client posts the scenario request to the Tachyon admin endpoint
- **THEN** the MCP response returns whether the scenario was accepted and how to observe the run

### Requirement: Advanced MCP Client Bindings
tachyon-client SHALL provide the HTTP request bindings required by the advanced MCP tools.

#### Scenario: MCP tools use client bindings
- **GIVEN** any advanced MCP tool needs Tachyon runtime data or operations
- **WHEN** the MCP server invokes tachyon-client
- **THEN** tachyon-client performs the required admin request with the active connection token
- **AND** non-success admin responses are surfaced as explicit errors
