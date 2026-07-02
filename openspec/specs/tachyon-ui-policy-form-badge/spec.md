# tachyon-ui-policy-form-badge Specification

## Purpose
Document the retirement of the former policy-only badge pattern. Runtime-facing
panels must now either read and write real `IntegrityConfig` fields or be
removed from shell navigation.

## Requirements
### Requirement: Policy-only badge pattern is retired
Tachyon-UI SHALL NOT use `<tachyon-policy-form-badge>` to mark panels that write
configuration without displaying runtime state. Panels affecting `core-host`
runtime behavior SHALL use manifest-backed controllers, and panels without a
runtime-backed field SHALL be removed from navigation.

#### Scenario: Runtime-backed panels do not show a policy-only badge
- **WHEN** the operator opens Resilience, Identity, Routing, AI, Storage, or Observability
- **THEN** the panel header does not contain `<tachyon-policy-form-badge>`
- **AND** any editable runtime field is read from `get_manifest_config` or a live admin API

#### Scenario: Legacy policy-only panels are not routed
- **WHEN** the application shell renders navigation
- **THEN** the former RBAC policy form, Fleet policy form, Supply Chain policy form, and RoutingDashboard legacy form are absent
- **AND** operators use the runtime-backed Users & Groups, bundle apply, Routing, or manifest-backed Resilience workflows instead
