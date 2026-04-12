## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL treat `integrity.lock` as the signed source of truth for each configured route, storing the normalized route path, an execution role of `user` or `system`, a logical service `name`, a semantic `version`, optional dependency constraints, optional `allowed_secrets`, route-scaling fields `min_instances` plus `max_concurrency`, and optional route volume mounts containing `host_path`, `guest_path`, and `readonly`.

#### Scenario: Host consumes a sealed manifest with explicit route SemVer metadata
- **WHEN** `core-host` starts with a signed manifest whose `/api/faas-a` entry declares `name = "faas-a"`, `version = "2.0.0"`, and a dependency map containing `faas-b = "^3.1.0"`
- **THEN** integrity validation succeeds
- **AND** the runtime preserves the normalized route path
- **AND** the runtime loads the declared service identity and dependency constraints from the signed manifest

#### Scenario: Host loads an older sealed manifest without SemVer route metadata
- **WHEN** `core-host` starts with a sealed manifest whose route entries omit `name`, `version`, and `dependencies`
- **THEN** integrity validation still succeeds
- **AND** the host defaults `name` from the route path
- **AND** the host defaults `version` to `0.0.0`
- **AND** the host defaults the dependency map to empty

## REMOVED Requirements

### Requirement: Signer CLI produces a sealed integrity manifest
**Reason**: The workspace no longer contains the `tachyon-cli` manifest-generation crate referenced by this requirement.
**Migration**: Provide a valid signed `integrity.lock` through dedicated manifest tooling before compiling or running `core-host`.

### Requirement: The workspace provides a desktop manifest generator backed by the renamed UI crate
**Reason**: `tachyon-ui` is now a pure Tauri desktop shell that does not expose `generate`.
**Migration**: Keep manifest production outside `tachyon-ui` and supply the resulting signed `integrity.lock` as an input artifact.
