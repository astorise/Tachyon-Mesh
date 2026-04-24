## MODIFIED Requirements

### Requirement: The UI blocks dashboard access until a node connection succeeds
The desktop UI SHALL display a full-screen authentication gateway until the operator completes either the staged login flow or the invite-only signup and enrollment flow successfully.

#### Scenario: The operator switches from login to invite signup
- **WHEN** the operator chooses the desktop action to register with an invite token
- **THEN** the auth gateway transitions from the login form to a staged signup wizard
- **AND** the dashboard remains blocked until signup completes successfully

#### Scenario: Invite signup completes and unlocks the dashboard
- **WHEN** the operator validates an invite token, submits the required profile fields, and confirms the first TOTP code
- **THEN** the desktop frontend stores the returned authenticated session through the existing Tauri/client bridge
- **AND** it unlocks dashboard navigation for the enrolled user
- **AND** it routes the user into the main dashboard without requiring a second login
