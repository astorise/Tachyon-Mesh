## Purpose

Define the shared integration pipeline that lets Tachyon UI and MCP stage, seal, and apply configuration changes against the real Tachyon core.

## Requirements

### Requirement: UI credentials are not persisted in browser localStorage
Tachyon UI SHALL initialize Tauri Stronghold and route remembered credential persistence through native commands instead of writing passwords to browser `localStorage`.

#### Scenario: Operator enables credential remembering
- **WHEN** the operator enables remember credentials during authentication
- **THEN** the UI calls a native credential persistence command
- **AND** browser `localStorage` does not store the password value

#### Scenario: Operator disables credential remembering
- **WHEN** the operator clears the remember credentials preference
- **THEN** the UI calls a native credential deletion command
- **AND** stale browser credential state is not used for password restoration

### Requirement: Validated UI configuration is staged for sealing
The Tauri `apply_configuration` command SHALL keep existing WIT validation and stage successfully validated payloads into the local overlay.

#### Scenario: Configuration validates successfully
- **WHEN** a configuration panel submits a valid domain payload
- **THEN** `apply_configuration` writes the payload to the local overlay
- **AND** returns `success=true`, `staged=true`, and `requiresSeal=true`

#### Scenario: Configuration validation fails
- **WHEN** a configuration panel submits an invalid payload
- **THEN** `apply_configuration` returns `success=false`
- **AND** it does not mark the payload as staged or requiring seal

### Requirement: Tauri can seal and apply pending overlays
Tachyon UI SHALL provide a `seal_and_apply_manifest` command that signs pending overlay state and POSTs the signed manifest to `/admin/manifest`.

#### Scenario: Seal and apply succeeds
- **WHEN** a Tachyon node connection is active and pending overlay state exists
- **THEN** `seal_and_apply_manifest` writes a signed `integrity.lock`
- **AND** POSTs the manifest to `/admin/manifest`
- **AND** returns the accepted `configVersion`

#### Scenario: Host rejects manifest
- **WHEN** the host returns a non-success status for `/admin/manifest`
- **THEN** the command returns an explicit error including the host response

### Requirement: Shell exposes pending Seal & Apply action
The Tachyon app shell SHALL show a global pending changes button when any configuration payload requires sealing.

#### Scenario: Configuration stage event occurs
- **WHEN** the frontend receives an `apply_configuration` response with `requiresSeal=true`
- **THEN** the shell renders a visible `Pending Changes: Seal & Apply` action

#### Scenario: Operator applies pending changes
- **WHEN** the operator clicks the pending changes action
- **THEN** the shell invokes `seal_and_apply_manifest`
- **AND** shows success or failure through the global toast manager

### Requirement: Core host exposes runtime metrics to admin clients
The core host SHALL expose `GET /admin/metrics` as an authenticated admin endpoint backed by the runtime telemetry snapshot.

#### Scenario: Admin requests runtime metrics
- **WHEN** an authenticated admin client requests `/admin/metrics`
- **THEN** the host returns JSON containing source, error rate, latency, and queue depth fields
- **AND** the request does not fall through to the FaaS route fallback

### Requirement: Core host exposes shadow divergence reports to admin clients
The core host SHALL expose `GET /admin/shadow/diffs` as an authenticated admin endpoint for recent shadow traffic divergence reports.

#### Scenario: Admin requests shadow diffs
- **WHEN** an authenticated admin client requests `/admin/shadow/diffs`
- **THEN** the host returns a JSON array of divergence records
- **AND** the endpoint remains available even when no divergences have been recorded

### Requirement: Core host accepts chaos scenario requests
The core host SHALL expose `POST /admin/chaos/scenarios` as an authenticated admin endpoint for supported chaos harness scenarios.

#### Scenario: Admin starts a supported chaos scenario
- **WHEN** an authenticated admin client posts a supported scenario payload
- **THEN** the host returns an accepted JSON outcome
- **AND** invalid scenario names or excessive durations are rejected as caller errors

### Requirement: MCP can seal and apply manifests
The Tachyon MCP server SHALL expose tools that allow agents to seal local overlays and apply sealed manifests.

#### Scenario: Agent seals overlay
- **WHEN** an agent calls `tachyon_seal_overlay`
- **THEN** the MCP server seals the local overlay into `integrity.lock`
- **AND** returns the new config version

#### Scenario: Agent applies sealed manifest
- **WHEN** an agent calls `tachyon_apply_manifest`
- **THEN** the MCP server posts the current `integrity.lock` manifest to `/admin/manifest`
- **AND** reports host rejection errors explicitly

### Requirement: Core host rewrites outbound secret placeholders at the WASI HTTP boundary
The core host SHALL keep real egress secrets outside guest linear memory by allowing guests to send `tachyon:secret:<uuid>` placeholders and replacing those placeholders only in the host-owned outbound HTTP path.

#### Scenario: Allowed host receives plaintext secret
- **GIVEN** a secret placeholder is registered with `api.openai.com` in its allowed hosts
- **WHEN** a guest outbound HTTP request targets `api.openai.com` with the placeholder in a header or UTF-8 body
- **THEN** the host replaces the placeholder with the plaintext secret before dispatching the request

#### Scenario: Disallowed host receives honeypot placeholder
- **GIVEN** a secret placeholder is registered only for `api.openai.com`
- **WHEN** a guest outbound HTTP request targets `evil.test` with the placeholder in a header or UTF-8 body
- **THEN** the host leaves the placeholder unchanged
- **AND** the plaintext secret is not exposed to the disallowed destination
