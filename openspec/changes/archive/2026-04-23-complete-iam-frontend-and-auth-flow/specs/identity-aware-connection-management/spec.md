## MODIFIED Requirements

### Requirement: The desktop client keeps an authenticated connection profile
The desktop client SHALL hold a runtime connection profile with the node URL, operator name, bearer token, and optional mTLS identity bytes used by the AuthN gateway.

#### Scenario: The operator submits the AuthN gateway form
- **WHEN** the operator enters a mesh URL, username, bootstrap secret or admin token, and an optional mTLS identity bundle
- **THEN** the frontend validates the connection through the existing client/Tauri bridge before unlocking the session
- **AND** it stores the resulting authenticated connection profile for subsequent admin requests
- **AND** it retains the submitted operator name for IAM rendering in the desktop shell

### Requirement: The UI blocks dashboard access until a node connection succeeds
The desktop UI SHALL display a full-screen authentication gateway until the operator completes the staged login and MFA flow successfully.

#### Scenario: The operator advances from login to MFA
- **WHEN** the login step succeeds
- **THEN** the frontend animates the AuthN gateway from the credential form to the MFA form
- **AND** the dashboard remains blocked while the MFA step is pending

#### Scenario: The operator completes MFA
- **WHEN** the operator submits a valid-looking MFA code from the AuthN gateway
- **THEN** the frontend fades out the overlay with GSAP
- **AND** it unlocks dashboard navigation for the active authenticated session
