# control-plane-rbac Specification

## ADDED Requirements

### Requirement: Configuration API Mutations MUST be RBAC-Authorized
The `system-faas-config-api` SHALL intercept all incoming REST, gRPC, and MCP requests, extract the caller's identity (via JWT/OIDC claims), and evaluate their intent against the declarative `rbac-configuration` state.

#### Scenario: Unauthorized domain access
- **GIVEN** a user mapped to the `tenant-developer` role via OIDC groups
- **WHEN** the user attempts to `UPDATE` a payload targeting the `config-hardware` domain (e.g., enabling eBPF XDP)
- **THEN** the API's `evaluate-access` function returns `false`
- **AND** the API rejects the request with a `403 Forbidden` without processing the payload.

### Requirement: Role Bindings MUST support Tenant-level ACL Isolation
The authorization engine SHALL support `resource_selectors` on Role Bindings to ensure operators can only mutate configurations that apply to their designated fleets or namespaces.

#### Scenario: Cross-tenant modification attempt
- **GIVEN** a developer bound to the `tenant-developer` role restricted to `tenant: finance` labels
- **WHEN** the developer attempts to update a `config-routing` resource labeled `tenant: marketing`
- **THEN** the authorization engine rejects the intent due to label mismatch, preventing cross-tenant infrastructure manipulation.
