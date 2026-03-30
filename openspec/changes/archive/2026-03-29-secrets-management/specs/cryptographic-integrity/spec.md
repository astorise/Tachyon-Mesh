## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a structured entry containing the normalized route path, an execution role of `user` or `system`, and an optional `allowed_secrets` list when a user route is permitted to read named secrets from the host vault.

#### Scenario: Generating a manifest with both user and system routes
- **WHEN** a developer runs `tachyon-cli generate` with regular routes and at least one privileged telemetry route
- **THEN** the canonical configuration payload includes every route as an object with `path` and `role`
- **AND** regular guest routes are sealed with role `user`
- **AND** privileged telemetry routes are sealed with role `system`

#### Scenario: Generating a manifest with a secret-enabled user route
- **WHEN** a developer runs `tachyon-cli generate --route /api/guest-example --secret-route /api/guest-example=DB_PASS --memory 64`
- **THEN** the canonical configuration payload includes `/api/guest-example` with role `user`
- **AND** that route entry includes `allowed_secrets` containing `DB_PASS`
- **AND** routes without secret grants omit or leave empty `allowed_secrets`
