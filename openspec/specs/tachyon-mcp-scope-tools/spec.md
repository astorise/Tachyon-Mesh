# tachyon-mcp-scope-tools Specification

## Purpose
TBD - created by archiving change tachyon-mcp-scope-tools. Update Purpose after archive.
## Requirements
### Requirement: MCP tool tachyon_get_scope_denials returns per-category denial summary
The MCP server SHALL expose a `tachyon_get_scope_denials` tool that queries `GET /admin/metrics` and returns a structured denial summary for a given deployment route or for all routes.

#### Scenario: Agent queries denial summary for a specific route
- **GIVEN** an MCP client calls `tachyon_get_scope_denials` with `route_path: "/api/billing"`
- **WHEN** the tool executes
- **THEN** the response includes `route_path`, `scope_denial_total`, and a `by_category` object with per-category counts from the prometheus `faas_scope_denials_total{deployment="/api/billing"}` labels
- **AND** the response includes `allow_all: true/false` indicating whether the route uses allow-all scopes

#### Scenario: Agent queries denial summary with no route filter
- **GIVEN** an MCP client calls `tachyon_get_scope_denials` with no arguments
- **WHEN** the tool executes
- **THEN** the response includes a `routes` array with one summary object per route that has at least one denial
- **AND** routes with zero denials are omitted from the array

#### Scenario: Node is unreachable
- **WHEN** `GET /admin/metrics` fails or times out
- **THEN** the tool returns a JSON-RPC error with code `-32001` (cluster unreachable)

### Requirement: MCP tool tachyon_set_route_scopes applies a scopes block to a route
The MCP server SHALL expose a `tachyon_set_route_scopes` tool that accepts a `route_path` and a `scopes` object, merges it into the current manifest, and submits via `POST /admin/manifest`.

#### Scenario: Agent applies minimal kv scope to a route
- **GIVEN** an MCP client calls `tachyon_set_route_scopes` with `route_path: "/api/billing"` and `scopes: {"kv": ["billing/**"]}`
- **WHEN** the tool executes
- **THEN** the tool fetches the current manifest via `GET /admin/manifest`
- **AND** merges the `scopes:` block into the target route entry
- **AND** submits the updated manifest via `POST /admin/manifest`
- **AND** returns `{"success": true, "route_path": "/api/billing", "scopes_applied": {"kv": ["billing/**"]}}`

#### Scenario: Agent uses dry_run mode to preview the manifest change
- **GIVEN** an MCP client calls `tachyon_set_route_scopes` with `dry_run: true`
- **WHEN** the tool executes
- **THEN** the tool returns the would-be manifest payload as `{"dry_run": true, "manifest_preview": {...}}`
- **AND** no state is written to the node

#### Scenario: Route not found in manifest
- **WHEN** the `route_path` argument does not match any route in the current manifest
- **THEN** the tool returns a JSON-RPC error `-32602` (invalid params) with message "route not found: /api/billing"

#### Scenario: Rate limit prevents rapid manifest churn
- **WHEN** `tachyon_set_route_scopes` is called more than once within 60 seconds
- **THEN** the second call returns a JSON-RPC error `-32002` with `retry_after_ms`

### Requirement: MCP tool tachyon_suggest_scopes produces a minimal scope block from denial data
The MCP server SHALL expose a `tachyon_suggest_scopes` tool that reads current denial counters and the current manifest, then produces a recommended `scopes:` block for a given route path in YAML format.

#### Scenario: Agent requests scope suggestion for a route with kv and secrets denials
- **GIVEN** a route `/api/billing` has `kv` denials for `billing/**` and `secrets` denials for `db/prod/*`
- **WHEN** an MCP client calls `tachyon_suggest_scopes` with `route_path: "/api/billing"`
- **THEN** the tool returns a structured suggestion including:
  - `current_state: "allow-all"` or `"partially-scoped"`
  - `denial_summary` with per-category counts
  - `suggested_scopes` as a YAML snippet ready to paste into the manifest
  - `rationale` explaining each suggested pattern
- **AND** the suggestion includes a note: "Patterns are derived from observed denied arguments; review before applying."

#### Scenario: Route has no denials and is allow-all
- **WHEN** a route is allow-all with zero denials recorded
- **THEN** the tool returns `suggested_scopes: null` and `rationale: "No denials recorded. The deployment may not have received traffic yet, or all calls are passing allow-all. Add explicit scopes based on known usage patterns."`

#### Scenario: Suggestion is for informational purposes only
- **WHEN** `tachyon_suggest_scopes` is called
- **THEN** the tool MUST NOT apply any changes to the manifest
- **AND** the response includes `"apply_with": "tachyon_set_route_scopes"` to guide the agent toward the mutation tool

