## ADDED Requirements

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
