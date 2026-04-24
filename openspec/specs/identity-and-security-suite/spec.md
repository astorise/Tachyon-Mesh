# identity-and-security-suite Specification

## Purpose
TBD - created by archiving change identity-and-security-suite. Update Purpose after archive.
## Requirements
### Requirement: Recovery codes are generated once and stored only as hashes
The AuthN component SHALL generate recovery codes in plaintext once, persist only hashed values, and associate them with the requesting username.

#### Scenario: An operator initializes recovery codes
- **WHEN** the desktop client requests recovery codes for the administrator
- **THEN** the AuthN component generates ten codes in the `TCHN-XXXXX-XXXXX` format
- **AND** it stores only their hashes in persistent auth state
- **AND** it returns the plaintext codes exactly once in the response

### Requirement: Recovery codes can mint a short-lived emergency session
The AuthN component SHALL burn a recovery code immediately when it is redeemed and return a short-lived JWT for emergency recovery.

#### Scenario: A valid recovery code is redeemed
- **WHEN** a caller submits a stored recovery code for a username
- **THEN** the matching hash is removed from storage
- **AND** the AuthN component returns a temporary authenticated JWT

#### Scenario: An invalid or reused recovery code is redeemed
- **WHEN** the submitted recovery code does not match any stored hash
- **THEN** the request is rejected
- **AND** no new JWT is issued

### Requirement: Personal access tokens are issued once and stored only as hashes
The AuthN component SHALL mint Personal Access Tokens in plaintext once, persist only hashed token material, and bind each token to the owning subject plus the requested scopes and expiry.

#### Scenario: An operator issues a PAT
- **WHEN** an authenticated administrator requests a new PAT with a name, scope list, and TTL
- **THEN** the AuthN component generates a random plaintext PAT
- **AND** it stores only the token hash with the name, scopes, owner, and expiry metadata
- **AND** it returns the plaintext token exactly once in the response

#### Scenario: An expired PAT is presented
- **WHEN** a caller presents a PAT after its expiry time
- **THEN** AuthN rejects the token
- **AND** the host responds with `401 Unauthorized`

### Requirement: The desktop UI exposes a personal account security view
The desktop UI SHALL expose a dedicated `My Account` view where the currently connected operator can issue PATs and trigger personal security actions without reusing the shared identity-management panel.

#### Scenario: The operator opens My Account
- **WHEN** the operator selects the `My Account` sidebar entry
- **THEN** the desktop frontend switches to `#view-account`
- **AND** it renders PAT issuance controls and personal security actions for the active administrative connection

#### Scenario: The operator generates a PAT from the desktop UI
- **WHEN** the operator submits a PAT name from the `My Account` view
- **THEN** the frontend invokes the shared Tauri command for PAT issuance
- **AND** the returned token is rendered once in the account view

### Requirement: First-run onboarding forces the operator to save recovery codes
The desktop UI SHALL block completion of the first-run security modal until recovery codes are displayed and explicitly saved.

#### Scenario: Recovery codes are generated during onboarding
- **WHEN** the operator advances from the QR step to the recovery-code step
- **THEN** the UI requests recovery codes through the shared client layer
- **AND** it renders the returned codes in the modal
- **AND** it blocks completion until the operator downloads the text bundle

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

### Requirement: Invite-only signup tokens stage users before activation
The AuthN component SHALL accept dedicated registration tokens that expire within 24 hours, stage the submitted user profile, and keep the account inactive until the first TOTP code is verified successfully.

#### Scenario: A valid registration token starts signup
- **WHEN** a caller submits a signed registration token whose declared use is `registration`
- **THEN** the AuthN component validates the token signature and expiry
- **AND** it exposes the pre-assigned roles and scopes carried by that invite

#### Scenario: A staged signup returns a TOTP provisioning URI
- **WHEN** a caller submits a valid registration token plus the first name, last name, username, and password for the invited user
- **THEN** the AuthN component persists the staged enrollment state without activating the account
- **AND** it returns a session identifier plus an `otpauth://` provisioning URI for the pending user

#### Scenario: Enrollment completes only after the first valid TOTP code
- **WHEN** a caller submits the staged enrollment session identifier and a valid 6-digit TOTP code
- **THEN** the AuthN component activates the user record
- **AND** it invalidates the registration token so it cannot be reused
- **AND** it returns an authenticated JWT for the newly enrolled user

#### Scenario: Invalid or expired signup state is rejected
- **WHEN** a caller submits an expired registration token, a reused token, an expired staged session, or an invalid TOTP code
- **THEN** the AuthN component rejects the request
- **AND** it does not activate the user

