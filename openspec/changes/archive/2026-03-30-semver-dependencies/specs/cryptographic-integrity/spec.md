## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a structured entry
containing the normalized route path, an execution role of `user` or `system`, a logical service
`name`, a semantic `version`, optional dependency constraints, optional `allowed_secrets`,
route-scaling fields `min_instances` plus `max_concurrency`, and optional route volume mounts
containing `host_path`, `guest_path`, and `readonly`.

#### Scenario: Generating a manifest with explicit route SemVer metadata
- **WHEN** a developer runs `tachyon-cli generate --route /api/faas-a --route-name /api/faas-a=faas-a --route-version /api/faas-a=2.0.0 --route-dependency /api/faas-a=faas-b@^3.1.0 --memory 64`
- **THEN** the canonical configuration payload includes `/api/faas-a`
- **AND** the same route entry includes `name = "faas-a"` and `version = "2.0.0"`
- **AND** the same route entry includes a dependency map containing `faas-b = "^3.1.0"`
- **AND** the route remains normalized before it is signed

#### Scenario: Loading an older manifest without SemVer route metadata
- **WHEN** `core-host` starts with a sealed manifest whose route entries omit `name`, `version`, and `dependencies`
- **THEN** integrity validation still succeeds
- **AND** the host defaults `name` from the route path
- **AND** the host defaults `version` to `0.0.0`
- **AND** the host defaults the dependency map to empty
