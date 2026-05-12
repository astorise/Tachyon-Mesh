## ADDED Requirements

### Requirement: Guided tour explains Atomic Seal & Apply
The Tachyon guided tour SHALL include an Atomic Seal & Apply step after the registry overview that targets the main pending Seal & Apply action.

#### Scenario: Operator reaches the Seal & Apply tour step
- **WHEN** the guided tour advances past the registry overview
- **THEN** it shows the title `Atomic Seal & Apply`
- **AND** it explains visual diffs, atomic signing, and Step-Up MFA
- **AND** it highlights the pending Seal & Apply action when available

### Requirement: Connection store persists MFA recency
The Tachyon UI connection store SHALL hydrate and persist `lastMfaTimestamp` through local browser storage so a page reload does not immediately discard a valid Step-Up MFA grace period.

#### Scenario: MFA timestamp is updated
- **WHEN** the UI records a new `lastMfaTimestamp`
- **THEN** the value is saved to local storage
- **AND** the in-memory connection store is updated

#### Scenario: UI reloads during MFA grace period
- **WHEN** the connection store initializes after a reload
- **THEN** it reads the stored timestamp
- **AND** uses `0` if storage is unavailable or invalid

### Requirement: Dynamic panel HTML escapes untrusted values
Tachyon UI panels that render dynamic values with `innerHTML` SHALL escape values from host data before interpolation.

#### Scenario: Topology source contains HTML
- **GIVEN** the topology source returned by the host contains HTML metacharacters
- **WHEN** `TachyonTopologyPanel` renders the source banner
- **THEN** those metacharacters are escaped before insertion into `innerHTML`

#### Scenario: User or group identifiers contain HTML
- **GIVEN** IAM user or group names contain HTML metacharacters
- **WHEN** `TachyonUsersPanel` renders table rows or action attributes
- **THEN** those identifiers are escaped before insertion into `innerHTML`
