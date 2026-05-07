# iam-webcomponent Specification

## Purpose
TBD - created by archiving change tachyon-ui-webcomponents-phase1. Update Purpose after archive.
## Requirements
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

### Requirement: Tachyon UI MUST expose IAM as an isolated Web Component
The Tachyon UI SHALL render the authentication and invite-token enrollment workflow through a native `<tachyon-iam>` custom element using an open Shadow DOM and component-scoped styles.

#### Scenario: IAM component boots in isolation
- **GIVEN** the Tachyon UI page is loaded
- **WHEN** the browser upgrades custom elements
- **THEN** `<tachyon-iam>` is defined and renders the login workflow inside its Shadow DOM
- **AND** the component styling does not depend on global DOM selectors.

### Requirement: IAM Web Component MUST publish authentication state via DOM events
The IAM component SHALL emit `iam:authenticated` after a successful Tauri-backed authentication or enrollment flow and SHALL emit `iam:error` for handled failures.

#### Scenario: Successful login emits authentication details
- **GIVEN** valid credentials are submitted in `<tachyon-iam>`
- **WHEN** the Rust backend accepts the authentication request
- **THEN** the component emits `iam:authenticated`
- **AND** the event payload includes the user, role, and token fields.

#### Scenario: Failed login is handled without panics
- **GIVEN** invalid credentials are submitted in `<tachyon-iam>`
- **WHEN** the backend rejects the request
- **THEN** the component displays an inline error
- **AND** the component emits `iam:error`.
