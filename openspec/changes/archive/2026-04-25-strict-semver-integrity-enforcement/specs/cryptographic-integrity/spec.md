## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL treat `integrity.lock` as the signed source of truth for each configured route,
storing the normalized route path, an execution role of `user` or `system`, a logical service
`name`, a semantic `version`, optional dependency constraints, optional `allowed_secrets`,
route-scaling fields `min_instances` plus `max_concurrency`, and optional route volume mounts
containing `host_path`, `guest_path`, and `readonly`.

#### Scenario: Host consumes a sealed manifest with explicit route SemVer metadata
- **WHEN** `core-host` starts with a signed manifest whose `/api/faas-a` entry declares `name = "faas-a"`, `version = "2.0.0"`, and a dependency map containing `faas-b = "^3.1.0"`
- **THEN** integrity validation succeeds
- **AND** the runtime preserves the normalized route path
- **AND** the runtime loads the declared service identity and dependency constraints from the signed manifest

#### Scenario: Host rejects a sealed manifest whose route SemVer metadata is incomplete
- **WHEN** `core-host` starts with a sealed manifest whose route entry omits `version` or `dependencies`
- **THEN** integrity validation fails with `ERR_INTEGRITY_SCHEMA_VIOLATION`
- **AND** the host aborts startup before binding the HTTP server
