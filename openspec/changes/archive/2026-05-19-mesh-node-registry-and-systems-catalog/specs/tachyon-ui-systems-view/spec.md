## ADDED Requirements

### Requirement: Dedicated Systems route

Tachyon-UI SHALL expose a `systems` route registered in `tachyon-ui/src/registry/ComponentRegistry.ts` that mounts a `<tachyon-systems-panel>` custom element. The route MUST appear in the sidebar navigation in the platform section, after the `nodes` route.

#### Scenario: Systems route is listed in navigation

- **WHEN** the operator opens the application shell after authentication
- **THEN** the sidebar contains a "Systems" entry that activates the `systems` route
- **AND** the corresponding panel mounts under `<tachyon-systems-panel>`

### Requirement: Catalog with status

`<tachyon-systems-panel>` SHALL fetch the static catalog via `list_registered_systems` and the dynamic deployed list via `list_deployed_systems` on mount, then render a unified table with one row per `RegisteredSystem`. Each row MUST display: slug, catalog version, status (one of `not-deployed`, `deployed`, `version-drift`), and the number of host nodes running it.

#### Scenario: Catalog row reflects deployment status

- **GIVEN** the static catalog lists 35 systems
- **AND** 12 of them appear in `list_deployed_systems` with matching versions
- **AND** 2 of them appear in `list_deployed_systems` with mismatched versions
- **WHEN** the panel mounts
- **THEN** the table contains exactly 35 rows
- **AND** 12 rows display `status = "deployed"` with the matching host-node count
- **AND** 2 rows display `status = "version-drift"` with a warning indicator
- **AND** the remaining 21 rows display `status = "not-deployed"` with host-node count 0

#### Scenario: Empty deployed list

- **GIVEN** `list_registered_systems` returns 35 entries
- **AND** `list_deployed_systems` returns an empty array
- **WHEN** the panel mounts
- **THEN** every row displays `status = "not-deployed"`
- **AND** no row displays a warning indicator

### Requirement: Drill-in showing host nodes per system

The panel SHALL allow the operator to expand a row to see the list of `node_id`s currently running that system, with per-node version when available.

#### Scenario: Expand row shows node list

- **GIVEN** `system-faas-gateway` is reported as deployed on three nodes (A, B, C)
- **WHEN** the operator expands the `system-faas-gateway` row
- **THEN** the expanded section lists `A`, `B`, `C`
- **AND** each entry shows the per-node version retrieved from `DeployedSystem.node_versions`

#### Scenario: Expand row with no nodes shows placeholder

- **WHEN** the operator expands a row whose `status = "not-deployed"`
- **THEN** the expanded section shows the message "This system ships with Tachyon but is not currently active on any enrolled node"
- **AND** no action button is rendered (this change is read-only)

### Requirement: Read-only contract

This view MUST NOT expose any control that installs, activates, deactivates, or configures a system. The activate/install flow is explicitly out of scope for this change and SHALL be introduced by a subsequent change.

#### Scenario: No mutating control is rendered

- **WHEN** the panel mounts and renders 35 catalog rows
- **THEN** no row contains a button or form that triggers a mutating Tauri command
- **AND** no row contains a "deploy" or "activate" affordance
