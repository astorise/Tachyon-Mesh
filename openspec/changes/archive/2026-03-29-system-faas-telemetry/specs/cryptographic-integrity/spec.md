## ADDED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a structured entry containing the normalized route path and an execution role of `user` or `system`.

#### Scenario: Generating a manifest with both user and system routes
- **WHEN** a developer runs `tachyon-cli generate` with regular routes and at least one privileged telemetry route
- **THEN** the canonical configuration payload includes every route as an object with `path` and `role`
- **AND** regular guest routes are sealed with role `user`
- **AND** privileged telemetry routes are sealed with role `system`

### Requirement: Host rejects ambiguous or invalid sealed route metadata
`core-host` SHALL normalize sealed route paths, reject duplicates, and refuse to start if any sealed route metadata is invalid.

#### Scenario: Startup aborts after duplicate route metadata
- **WHEN** the embedded configuration payload contains the same normalized route more than once
- **THEN** `core-host` fails integrity validation before serving traffic
- **AND** the error reports that the sealed route metadata is ambiguous
