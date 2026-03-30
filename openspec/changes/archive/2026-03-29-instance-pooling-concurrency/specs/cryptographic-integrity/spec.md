## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a structured entry
containing the normalized route path, an execution role of `user` or `system`, optional
`allowed_secrets`, and route-scaling fields `min_instances` plus `max_concurrency`.

#### Scenario: Generating a manifest with explicit route scaling
- **WHEN** a developer runs `tachyon-cli generate --route /api/guest-example --route-scale /api/guest-example=2:16 --memory 64`
- **THEN** the canonical configuration payload includes `/api/guest-example` with `min_instances = 2`
- **AND** the same route entry includes `max_concurrency = 16`
- **AND** the route remains normalized before it is signed

#### Scenario: Loading an older manifest without scaling fields
- **WHEN** `core-host` starts with a sealed manifest whose route entries omit `min_instances` and `max_concurrency`
- **THEN** integrity validation still succeeds
- **AND** the host defaults `min_instances` to `0`
- **AND** the host defaults `max_concurrency` to `100`
