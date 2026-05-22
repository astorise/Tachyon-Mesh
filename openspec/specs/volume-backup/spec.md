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

