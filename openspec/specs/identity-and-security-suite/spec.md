# identity-and-security-suite Specification

## Purpose
TBD - created by archiving change identity-and-security-suite. Update Purpose after archive.
## Requirements
### Requirement: Recovery codes are generated once and stored only as hashes
The auth component SHALL generate recovery codes in plaintext once, persist only hashed values, and associate them with the requesting username.

#### Scenario: An operator initializes recovery codes
- **WHEN** the desktop client requests recovery codes for the administrator
- **THEN** the auth component generates ten codes in the `TCHN-XXXXX-XXXXX` format
- **AND** it stores only their hashes in persistent auth state
- **AND** it returns the plaintext codes exactly once in the response

### Requirement: Recovery codes can mint a short-lived emergency session
The auth component SHALL burn a recovery code immediately when it is redeemed and return a short-lived JWT for emergency recovery.

#### Scenario: A valid recovery code is redeemed
- **WHEN** a caller submits a stored recovery code for a username
- **THEN** the matching hash is removed from storage
- **AND** the auth component returns a temporary authenticated JWT

#### Scenario: An invalid or reused recovery code is redeemed
- **WHEN** the submitted recovery code does not match any stored hash
- **THEN** the request is rejected
- **AND** no new JWT is issued

### Requirement: First-run onboarding forces the operator to save recovery codes
The desktop UI SHALL block completion of the first-run security modal until recovery codes are displayed and explicitly saved.

#### Scenario: Recovery codes are generated during onboarding
- **WHEN** the operator advances from the QR step to the recovery-code step
- **THEN** the UI requests recovery codes through the shared client layer
- **AND** it renders the returned codes in the modal
- **AND** it blocks completion until the operator downloads the text bundle

