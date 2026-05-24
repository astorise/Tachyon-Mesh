# tachyon-ui-route-config Specification

## Purpose
TBD - created by archiving change s3-faas-volumes. Update Purpose after archive.
## Requirements
### Requirement: Route detail view displays a Volumes panel listing all configured volumes
The Tachyon-UI SHALL provide a Volumes panel in the route detail view that lists all volumes configured for a route, distinguishing between Host, RAM, and S3 volume types.

#### Scenario: Volumes panel lists S3 volumes with metadata
- **WHEN** a user navigates to a route detail view for a route with one or more S3 volumes
- **THEN** the Volumes panel displays each S3 volume as a card showing: S3 URL (bucket + prefix), guest mount path, read-write or read-only mode
- **AND** the card includes a visual indicator distinguishing it from Host and RAM volumes

#### Scenario: Volumes panel shows empty state for routes without volumes
- **WHEN** a user navigates to a route detail view for a route with no volumes configured
- **THEN** the Volumes panel shows an empty state with a prompt to add a volume

### Requirement: Volumes panel allows adding an S3 volume via a configuration modal
The Volumes panel SHALL provide an "Add S3 Volume" action that opens a modal collecting the S3 URL, guest mount path, and read-only flag, then applies the change via the admin manifest API.

#### Scenario: User adds an S3 volume via the modal
- **WHEN** a user clicks "Add S3 Volume" in the Volumes panel
- **THEN** a modal opens with fields for S3 URL, guest path, and a read-only toggle
- **WHEN** the user submits valid values
- **THEN** the modal closes and the Volumes panel refreshes showing the new S3 volume card
- **AND** a success toast confirms the manifest was updated

#### Scenario: Invalid S3 URL is rejected in the modal
- **WHEN** a user enters a string that does not match `s3://bucket/prefix` in the S3 URL field
- **THEN** the field shows an inline validation error before submission
- **AND** the submit button remains disabled

### Requirement: Volumes panel allows removing an S3 volume
Each S3 volume card in the Volumes panel SHALL include a remove action that detaches the volume from the route after a confirmation prompt.

#### Scenario: User removes an S3 volume
- **WHEN** a user clicks "Remove" on an S3 volume card and confirms the prompt
- **THEN** the volume is removed from the route manifest
- **AND** the Volumes panel refreshes without the removed card
- **AND** a toast confirms the volume was detached

### Requirement: Route detail view exposes a Concurrency Policy panel with risk badges
The Tachyon-UI SHALL provide a Concurrency Policy panel in the route detail view that lets the operator configure the `concurrency`, `consistency`, and `coordination` modes, displaying a risk-level badge and a tooltip with a concrete failure scenario for each selection.

#### Scenario: Selecting a high-risk combination surfaces a red badge with tooltip
- **WHEN** an operator selects `concurrency.mode: "unrestricted"` and a shared volume with `consistency.write_mode: "last_write_wins"`
- **THEN** the panel displays a red `High Risk` badge next to the volume row
- **AND** hovering the badge shows a tooltip: "Concurrent invocations will silently overwrite each other's writes."

#### Scenario: Selecting a low-risk combination surfaces a green badge
- **WHEN** an operator selects `concurrency.mode: "mesh-singleton"` and `consistency.write_mode: "pessimistic_lock"`
- **THEN** the panel displays a green `Low Risk` badge
- **AND** the tooltip explains the latency trade-off: "All invocations serialize through a distributed lock; expect added latency under load."

#### Scenario: Panel hides incompatible combinations
- **WHEN** the operator selects `consistency.write_mode: "pessimistic_lock"`
- **AND** the route's `concurrency.mode` is `"unrestricted"`
- **THEN** the panel surfaces an inline warning that pessimistic locking only makes sense with singleton modes
- **AND** offers a one-click fix to switch concurrency to `mesh-singleton`

#### Scenario: Panel exposes data attributes for future simulation hook
- **WHEN** the panel renders any mode option
- **THEN** the option element carries a `data-sim-scenario="<scenario-id>"` attribute
- **AND** a future JS simulation script can attach to those attributes without modifying the panel

### Requirement: Route detail view includes a Scopes panel alongside Volumes and Concurrency panels
The route detail view SHALL render a Scopes panel (implemented by `tachyon-ui-scope-editor`) as a peer panel alongside the existing Volumes and Concurrency Policy panels.

#### Scenario: Scopes panel appears in the route detail tab layout
- **WHEN** a user navigates to the detail view of any FaaS route
- **THEN** a "Scopes" tab or section is visible in the panel layout alongside "Volumes" and "Concurrency"
- **AND** the Scopes panel is pre-expanded when the route resolves to allow-all (to prompt configuration)

#### Scenario: Scopes panel is collapsed by default for explicitly scoped routes
- **WHEN** the route has an explicit non-allow-all `scopes:` block
- **THEN** the Scopes panel is collapsed by default
- **AND** its header shows a summary chip count (e.g. "3 categories, 7 patterns")


## Requirements (s3-storage-backup)

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
