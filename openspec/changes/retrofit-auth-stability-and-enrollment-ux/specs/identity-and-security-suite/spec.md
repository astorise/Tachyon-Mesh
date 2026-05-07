## ADDED Requirements

### Requirement: Password login is staged until MFA finalization
The desktop AuthN flow SHALL submit username and password to a login staging operation, retain the returned MFA session identifier, and unlock the authenticated shell only after a successful finalization operation with a valid 6-digit MFA code.

#### Scenario: Password acceptance opens the MFA step
- **WHEN** the operator submits a valid node URL, username, and password
- **THEN** the desktop UI invokes the login staging operation
- **AND** it stores the returned MFA session identifier
- **AND** it switches to the MFA code step without unlocking the authenticated shell

#### Scenario: MFA finalization unlocks the shell
- **WHEN** the operator submits a valid 6-digit MFA code for the staged login session
- **THEN** the desktop UI invokes the login finalization operation
- **AND** the authenticated shell is unlocked only after the finalization response succeeds

#### Scenario: Malformed or missing MFA state blocks login
- **WHEN** password staging returns no MFA session identifier or the submitted MFA code is not six digits
- **THEN** the desktop UI rejects the flow
- **AND** the authenticated shell remains locked

### Requirement: Enrollment renders a QR-backed TOTP finalization step
The desktop enrollment flow SHALL render the `otpauth://` provisioning URI returned by staged signup as a scannable QR code and SHALL finalize enrollment only after the first valid 6-digit TOTP code is submitted.

#### Scenario: Staged enrollment renders QR and manual secret
- **WHEN** staged signup succeeds and returns a session identifier plus an `otpauth://` provisioning URI
- **THEN** the desktop UI renders a scannable QR code for the provisioning URI
- **AND** it displays the manual secret extracted from the provisioning URI
- **AND** it keeps the account inactive until finalization succeeds

#### Scenario: Enrollment finalization requires a six digit code
- **WHEN** the operator submits a TOTP value that is not exactly six digits
- **THEN** the desktop UI rejects the value before invoking finalization
- **AND** the staged account remains inactive

### Requirement: Enrollment includes its own Mesh Node URL input
The desktop enrollment flow SHALL render a Mesh Node URL input on the invite-token step and SHALL use that URL for invite validation, account staging, and enrollment finalization.

#### Scenario: Operator starts enrollment without visiting login first
- **WHEN** the operator opens `Register with Invite Token`
- **THEN** the invite-token step renders a Mesh Node URL input
- **AND** the operator can validate the invite token without returning to the signin step

#### Scenario: Missing enrollment URL is rejected
- **WHEN** the operator submits invite validation, account staging, or enrollment finalization without a Mesh Node URL
- **THEN** the desktop UI shows a validation error
- **AND** it does not invoke the backend enrollment command

#### Scenario: Login and enrollment URLs stay synchronized
- **WHEN** the operator edits the Mesh Node URL in either signin or enrollment
- **THEN** the other AuthN URL field is updated to the same value
- **AND** subsequent AuthN requests use the synchronized value

### Requirement: Desktop authentication can remember credentials with explicit opt-in
The desktop authentication UI SHALL expose an explicit workstation-local option to remember node URL, username, and password, while preserving MFA as the final authentication step.

#### Scenario: Remembered credentials are restored
- **WHEN** the operator previously opted in to saving credentials on the workstation
- **THEN** the desktop UI restores the node URL, username, and password fields on startup
- **AND** the remember option is shown as enabled

#### Scenario: Disabling remember clears saved credentials
- **WHEN** the operator disables the remember option
- **THEN** the desktop UI removes the saved credential bundle from local storage
- **AND** future startup does not prefill the password from that bundle

#### Scenario: Remembered password does not bypass MFA
- **WHEN** saved credentials are restored and the operator submits login
- **THEN** the desktop UI still performs the login staging operation
- **AND** the shell remains locked until MFA finalization succeeds
