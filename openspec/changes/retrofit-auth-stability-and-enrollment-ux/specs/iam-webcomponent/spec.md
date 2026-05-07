## ADDED Requirements

### Requirement: IAM web component mirrors stabilized login controls
The `<tachyon-iam>` component SHALL expose node URL, username, password, password visibility, remember-credentials, and MFA controls consistent with the desktop AuthN overlay.

#### Scenario: Component login requires MFA finalization
- **WHEN** credentials submitted through `<tachyon-iam>` are accepted by login staging
- **THEN** the component stores the MFA session identifier
- **AND** it emits `iam:authenticated` only after login finalization succeeds

#### Scenario: Component remember preference persists credentials
- **WHEN** the operator enables remember credentials and submits accepted credentials through `<tachyon-iam>`
- **THEN** the component persists node URL, username, and password in workstation-local storage
- **AND** future component initialization restores those fields

### Requirement: IAM web component enrollment is self-contained
The `<tachyon-iam>` component SHALL render a Mesh Node URL input on its invite-token enrollment step and use that URL for invite validation, account staging, and enrollment finalization.

#### Scenario: Component enrollment validates invite using enrollment URL
- **WHEN** the operator enters a Mesh Node URL and invite token in `<tachyon-iam>`
- **THEN** the component invokes invite validation with the enrollment URL
- **AND** it does not require the operator to return to the login step

#### Scenario: Component enrollment URL synchronizes with login URL
- **WHEN** the operator edits the node URL in either the component login step or the component enrollment step
- **THEN** the other node URL field is updated to the same value
- **AND** subsequent component AuthN requests use the synchronized value

#### Scenario: Component enrollment renders QR code for TOTP provisioning
- **WHEN** account staging returns an `otpauth://` provisioning URI
- **THEN** `<tachyon-iam>` renders a QR code for the URI
- **AND** it emits `iam:error` if the QR code cannot be generated
