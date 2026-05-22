## ADDED Requirements

### Requirement: MCP exposes a tool to recommend a concurrency policy from a usage pattern
The Tachyon MCP server SHALL provide a `recommend_concurrency_policy` tool that maps a declared usage pattern and requirements to a concrete `concurrency` + `consistency` + `coordination` configuration with a rationale and risk level.

#### Scenario: Recommendation for a stateful pattern returns mesh-singleton with locking
- **WHEN** an AI agent calls `recommend_concurrency_policy({ pattern: "stateful", requirements: { writes_shared_state: true } })`
- **THEN** the tool returns a JSON object with `concurrency.mode: "mesh-singleton"`, `consistency.write_mode: "pessimistic_lock"`, `coordination.write_isolation: "drain"`
- **AND** the response includes a `rationale` field explaining the choice in one sentence
- **AND** the response includes `risk_level: "low"` and a `trade_offs` array listing the latency cost

#### Scenario: Recommendation for an interactive pattern returns unrestricted defaults
- **WHEN** an AI agent calls `recommend_concurrency_policy({ pattern: "interactive" })`
- **THEN** the tool returns `concurrency.mode: "unrestricted"`, `consistency.write_mode: "last_write_wins"`, `coordination.mode: "per_node"`
- **AND** `risk_level: "low"` because no shared writable state is declared

#### Scenario: Recommendation for an etl pattern with conflict awareness returns optimistic_etag
- **WHEN** an AI agent calls `recommend_concurrency_policy({ pattern: "etl", requirements: { writes_shared_state: true, requires_ordering: false } })`
- **THEN** the tool returns `consistency.write_mode: "optimistic_etag"` and `coordination.mode: "mesh_leader"`
- **AND** `risk_level: "medium"` because optimistic conflicts can cause invocation failures
