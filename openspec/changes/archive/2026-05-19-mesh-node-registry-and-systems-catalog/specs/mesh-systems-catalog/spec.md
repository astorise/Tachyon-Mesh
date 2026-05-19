## ADDED Requirements

### Requirement: Static catalog of known systems

The build SHALL produce a static catalog of `system-faas-*` components shipped by this Tachyon version, derived from a `systems/manifest.toml` file checked into the repository. The catalog MUST include, for each system, a stable kebab-case slug, the crate name, the crate version, and a short human-readable description.

#### Scenario: Catalog enumerates every shipped system

- **GIVEN** `systems/manifest.toml` lists `N` entries
- **WHEN** `list_registered_systems()` is called on a freshly started host
- **THEN** the result contains exactly `N` `RegisteredSystem` entries
- **AND** each entry's `slug`, `crate_name`, `version`, and `description` match the manifest

#### Scenario: Workspace drift is detected at build time

- **GIVEN** `systems/manifest.toml` references a system slug not present in the workspace `Cargo.toml`
- **WHEN** the workspace is built
- **THEN** the build fails with an error pointing at the missing crate
- **AND** the error message identifies which slug is unmatched

### Requirement: Dynamic catalog of deployed systems

The catalog SHALL also expose a dynamic view of which `system-faas-*` components are currently active on the mesh, aggregated from each enrolled node's reported `active_systems` field. The dynamic view MUST include the slug, the running version, and the list of host `node_id`s where it is active.

#### Scenario: Deployed list aggregates across nodes

- **GIVEN** node `A` reports `["system-faas-gateway", "system-faas-prom"]` and node `B` reports `["system-faas-gateway"]`
- **WHEN** `list_deployed_systems()` is called
- **THEN** the result contains two entries
- **AND** the `system-faas-gateway` entry's `node_ids` contains both `A` and `B`
- **AND** the `system-faas-prom` entry's `node_ids` contains only `A`

#### Scenario: System with no active node is omitted

- **GIVEN** no enrolled node reports `system-faas-tee-runtime` in its `active_systems`
- **WHEN** `list_deployed_systems()` is called
- **THEN** the result does not contain an entry for `system-faas-tee-runtime`
- **AND** `list_registered_systems()` still lists it (it remains shipped, just not active)

### Requirement: Version reconciliation

When the version a node reports for a system differs from the version recorded in the static catalog, `list_deployed_systems()` SHALL surface both the catalog version and the per-node version so the operator can detect drift.

#### Scenario: Version skew is visible

- **GIVEN** the static catalog records `system-faas-gateway = "1.4.2"` and node `A` reports it as running version `"1.3.9"`
- **WHEN** `list_deployed_systems()` is called
- **THEN** the `system-faas-gateway` entry exposes `catalog_version = "1.4.2"`
- **AND** the entry's `node_versions` map associates `A` with `"1.3.9"`
- **AND** a `has_drift` flag on the entry is `true`
