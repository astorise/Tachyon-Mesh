# volume-backup Specification

## Purpose
TBD - created by archiving change concurrency-policies. Update Purpose after archive.
## Requirements
### Requirement: Volume backup scheduler honors coordination mode
The volume backup scheduler SHALL evaluate `backup_schedule.coordination` before triggering a scheduled backup, and SHALL execute the backup on exactly one node when `coordination` is `mesh_leader`.

#### Scenario: PerNode coordination preserves current behavior
- **WHEN** a volume has `backup_schedule.coordination: "per_node"` (or the legacy string form `backup_schedule: "0 3 * * *"`)
- **AND** the scheduler tick determines the cron is due
- **THEN** every active mesh node executes its own backup of its local volume directory
- **AND** each backup creates a distinct timestamped snapshot

#### Scenario: MeshLeader coordination prevents duplicate backups
- **WHEN** a volume has `backup_schedule.coordination: "mesh_leader"`
- **AND** the scheduler tick determines the cron is due
- **THEN** only the deterministically elected leader for that volume executes the backup
- **AND** other nodes skip the backup silently

#### Scenario: ManualOnly coordination disables the scheduler entirely
- **WHEN** a volume has `backup_schedule.coordination: "manual_only"`
- **THEN** the scheduler never triggers an automatic backup for that volume
- **AND** the volume can still be backed up via `POST /admin/volumes/backup` or MCP

### Requirement: Backup write_isolation drain pauses route admission during snapshot
The volume backup scheduler SHALL drain active invocations and pause new admissions for the owning route when `backup_schedule.write_isolation: "drain"` is set.

#### Scenario: Drain mode waits for active invocations before snapshot
- **WHEN** a backup with `write_isolation: "drain"` is triggered
- **AND** 3 invocations are currently executing on the route
- **THEN** the scheduler waits for all 3 to complete (with a configurable timeout)
- **AND** rejects new invocations with HTTP 503 during the drain window
- **AND** uploads the snapshot only after all 3 finish or the timeout elapses
- **AND** resumes admission once the upload completes


## Requirements (s3-storage-backup)

### Requirement: Admin API exposes on-demand backup and restore for route volumes
The core-host admin API SHALL provide endpoints to backup a route volume to S3 and restore a volume from a previously created snapshot, gated by PAT authentication.

#### Scenario: Operator triggers a volume backup
- **WHEN** an authenticated operator calls `POST /admin/volumes/backup` with `route_path` and `guest_path`
- **THEN** core-host uploads all files from the volume's local directory to `s3://bucket/<backup_prefix>/<route>/<guest_path>/<timestamp>/`
- **AND** returns a `BackupSnapshot` object with `snapshot_id`, `route_path`, `guest_path`, `timestamp_ms`, and `object_count`

#### Scenario: Operator triggers a volume restore
- **WHEN** an authenticated operator calls `POST /admin/volumes/restore` with `route_path`, `guest_path`, and `snapshot_id`
- **THEN** core-host downloads all objects from the snapshot S3 prefix back to the volume's local directory
- **AND** returns HTTP 204 on success

#### Scenario: Operator lists available snapshots
- **WHEN** an authenticated operator calls `GET /admin/volumes/backups?route_path=...&guest_path=...`
- **THEN** core-host returns a list of `BackupSnapshot` objects for that volume, ordered by `timestamp_ms` descending

#### Scenario: Volume not found during backup
- **WHEN** the specified `route_path` or `guest_path` does not match any sealed route volume
- **THEN** the endpoint returns HTTP 404 with a descriptive error message

### Requirement: integrity.lock accepts a backup_schedule field on volumes
The `IntegrityVolume` schema SHALL accept an optional `backup_schedule` field containing a cron expression. When set, core-host SHALL trigger automatic backups according to the schedule.

#### Scenario: Volume with valid cron schedule is backed up automatically
- **WHEN** a volume declares `backup_schedule: "0 3 * * *"` in the sealed manifest
- **AND** the scheduler tick determines the cron is due
- **THEN** core-host performs a backup equivalent to a manual `POST /admin/volumes/backup` call
- **AND** logs the backup completion at INFO level

#### Scenario: Invalid cron expression is rejected at validation
- **WHEN** an operator submits a manifest with `backup_schedule: "not-a-cron"`
- **THEN** integrity verification fails with a descriptive validation error naming the invalid schedule

### Requirement: S3 backup prefix is isolated from persistence prefix
The backup system SHALL write snapshots under a configurable prefix (`TACHYON_S3_BACKUP_PREFIX`, default `backups`) that is distinct from the S3 persistence prefix (`TACHYON_S3_PREFIX`) to prevent collisions.

#### Scenario: Backup and persistence use different S3 prefixes
- **WHEN** both S3 persistence and volume backup are active
- **THEN** backup objects are written under `<backup_prefix>/...`
- **AND** persistence objects are written under `<persistence_prefix>/...`
- **AND** a `list` of one prefix does not return objects from the other
