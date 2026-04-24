## ADDED Requirements

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

