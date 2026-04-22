## MODIFIED Requirements

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

## ADDED Requirements

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
