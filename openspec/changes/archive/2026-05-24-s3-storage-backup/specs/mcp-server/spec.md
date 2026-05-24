## ADDED Requirements

### Requirement: MCP exposes tools to backup, restore, and list snapshots for route volumes
The Tachyon MCP server SHALL provide three tools for managing volume backups, operating on the live admin API.

#### Scenario: backup_volume creates a snapshot
- **WHEN** an AI agent calls `backup_volume` with `route_path` and `guest_path`
- **THEN** the tool triggers `POST /admin/volumes/backup` and returns the resulting `BackupSnapshot` metadata

#### Scenario: restore_volume applies a snapshot
- **WHEN** an AI agent calls `restore_volume` with `route_path`, `guest_path`, and `snapshot_id`
- **THEN** the tool triggers `POST /admin/volumes/restore` and confirms successful restoration

#### Scenario: list_volume_backups returns available snapshots
- **WHEN** an AI agent calls `list_volume_backups` with `route_path` and `guest_path`
- **THEN** the tool returns a list of available snapshots ordered by date, newest first
- **AND** returns an empty list if no snapshots exist for that volume
