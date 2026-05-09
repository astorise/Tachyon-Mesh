# complete-auth-flow Specification

## Purpose
Secure and complete Tachyon-UI authentication flow covering remembered workstation credentials, custom CA handling, admin invite generation, step-up MFA, and guided onboarding.
## Requirements
### Requirement: Secure Auth Profile Persistence
Tachyon-UI SHALL persist remembered operator credentials and custom CA material through the native secure profile boundary instead of browser credential storage.

#### Scenario: Operator opts into workstation credential persistence
- **GIVEN** the login form remember toggle is enabled
- **WHEN** the operator authenticates or edits the persisted profile
- **THEN** Tachyon-UI stores the URL, username, password, PAT if available, and custom CA through native commands
- **AND** auth credentials are not written to browser `localStorage`

### Requirement: Admin Invite Generation
Tachyon-UI SHALL expose an admin-only invite generation panel that starts enrollment and renders both a manual token and QR code.

#### Scenario: Admin generates an invite
- **GIVEN** an authenticated admin is using the shell
- **WHEN** they request a new operator invite
- **THEN** the UI invokes the native enrollment command backed by `/admin/enrollment/start`
- **AND** the returned token is displayed with a QR code payload

### Requirement: Step-up Authentication
Sensitive write operations SHALL require a TOTP step-up confirmation, and
that confirmation SHALL be validated by the host through the existing
staged-login pipeline rather than by a local-only digit-count check.

#### Scenario: Step-up forwards the TOTP to the host
- **GIVEN** the sudo grace period has expired
- **WHEN** the operator submits a TOTP code via `<tachyon-mfa-prompt>`
- **THEN** Tachyon-UI calls `verify_session_totp`, which itself replays
  `authn_login` then `finalize_login` against the persisted operator
  profile
- **AND** the original sensitive command continues only when
  `finalize_login` accepts the TOTP code

#### Scenario: Step-up surfaces missing credentials
- **GIVEN** no operator profile is persisted in the workstation secure
  store
- **WHEN** the operator triggers a sensitive command
- **THEN** the step-up command returns an error explaining that step-up
  cannot complete without remembered credentials
- **AND** the original command is not executed

### Requirement: Custom CA Management
The authentication screen SHALL restore persisted custom CA material and provide visible controls to save or clear it.

#### Scenario: Operator manages custom CA material
- **GIVEN** a custom CA has been persisted on the workstation
- **WHEN** the authentication screen opens
- **THEN** the UI reports that a custom CA is loaded
- **AND** the operator can clear or replace the active certificate material

### Requirement: Auth Guided Tour
The guided tour SHALL highlight the critical auth, seal, apply, and observability path.

#### Scenario: First-run tour starts
- **GIVEN** the guided tour runs for an operator
- **WHEN** the steps are displayed
- **THEN** the tour includes login/setup context, the Seal & Apply pending state, and the observability metrics panel

