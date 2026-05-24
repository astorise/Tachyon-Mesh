## ADDED Requirements

### Requirement: Route detail view displays a Backups panel for volume snapshot management
The Tachyon-UI SHALL provide a Backups panel in the route detail view listing available S3 snapshots for each configured volume, with actions to create and restore snapshots.

#### Scenario: Backups panel lists snapshots for a volume
- **WHEN** a user navigates to a route detail view and selects a volume
- **THEN** the Backups panel displays a list of available snapshots with `snapshot_id`, date, and object count
- **AND** each snapshot has a "Restore" action

#### Scenario: User triggers a manual backup
- **WHEN** a user clicks "Backup now" in the Backups panel for a specific volume
- **THEN** a backup is created and the snapshot list refreshes showing the new entry
- **AND** a success toast confirms the snapshot was saved to S3

#### Scenario: Backups panel shows empty state when no snapshots exist
- **WHEN** a user navigates to the Backups panel for a volume with no prior snapshots
- **THEN** the panel shows an empty state with a prompt to create the first backup
