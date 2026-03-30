## MODIFIED Requirements

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a
structured entry containing the normalized route path, an execution role of
`user` or `system`, optional `allowed_secrets`, route-scaling fields
`min_instances` plus `max_concurrency`, and optional volume mounts containing
`host_path`, `guest_path`, and `readonly`.

#### Scenario: Generating a manifest with explicit route volumes
- **WHEN** a developer runs `tachyon-cli generate --route /api/guest-volume --volume /api/guest-volume=/tmp/tachyon_data:/app/data:rw --memory 64`
- **THEN** the canonical configuration payload includes `/api/guest-volume`
- **AND** the same route entry includes a volume with `host_path = /tmp/tachyon_data`
- **AND** the volume includes `guest_path = /app/data` and `readonly = false`
- **AND** the route remains normalized before it is signed

#### Scenario: Loading an older manifest without volume fields
- **WHEN** `core-host` starts with a sealed manifest whose route entries omit `volumes`
- **THEN** integrity validation still succeeds
- **AND** the host defaults the route volume list to empty
