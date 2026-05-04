# l7-resiliency Delta

## ADDED Requirements

### Requirement: Resiliency policies MUST be declarative and schema-driven
The `system-faas-config-api` SHALL expose a strict `config-resilience.wit` contract to allow Tachyon-UI and MCP clients to safely manipulate retries, timeouts, and fault injections without restarting the data-plane.

#### Scenario: Attaching a Chaos Engineering policy
- **GIVEN** an active production route handling database traffic
- **WHEN** the API receives a valid `ResilienceConfiguration` with a `chaos_injection` of 5% latency
- **THEN** the API validates the request and delegates the state change to the GitOps broker
- **AND** the `core-host` begins applying a 2500ms delay to 5% of the matched traffic asynchronously.

### Requirement: Shadow Traffic MUST be decoupled from the primary response
When a `shadow_traffic` policy is applied to a route, the runtime SHALL mirror the specified percentage of requests to the target group asynchronously, and the primary client response MUST NOT be delayed or impacted by the shadow target's performance.

#### Scenario: Mirroring traffic without delaying clients
- **GIVEN** a route has a `shadow_traffic` policy targeting an analysis backend
- **WHEN** a client request is served by the primary target
- **THEN** the runtime sends the mirrored request asynchronously
- **AND** returns the primary response without waiting for the shadow target.
