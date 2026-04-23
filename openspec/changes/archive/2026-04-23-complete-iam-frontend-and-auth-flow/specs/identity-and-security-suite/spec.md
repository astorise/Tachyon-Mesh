## ADDED Requirements

### Requirement: The desktop UI exposes a routed IAM dashboard for the active admin session
The desktop UI SHALL expose a dedicated `Identity` view that renders the active administrative subject, groups, endpoint posture, and supported security actions without leaving the authenticated shell.

#### Scenario: The operator opens Identity after authentication
- **WHEN** the operator selects the `Identity` sidebar entry after the AuthN gateway is dismissed
- **THEN** the desktop frontend switches to `#view-identity`
- **AND** it renders an IAM summary table for the active administrative session
- **AND** it shows the connected endpoint plus the current MFA and recovery-bundle posture

### Requirement: The IAM dashboard reuses existing security operations
The Identity view SHALL wire its security actions to the existing recovery-bundle APIs instead of inventing unsupported remote IAM CRUD operations.

#### Scenario: The operator regenerates the recovery bundle from Identity
- **WHEN** the operator triggers the IAM dashboard action to rotate recovery material for the active subject
- **THEN** the frontend invokes the shared Tauri/client command for recovery regeneration
- **AND** it renders the returned bundle in the Identity view
- **AND** it preserves the dedicated `My Account` PAT workflow for personal token issuance
