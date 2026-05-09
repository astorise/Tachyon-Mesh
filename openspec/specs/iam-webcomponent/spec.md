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

### Requirement: Single Enrollment Flow
The `<tachyon-iam>` component SHALL expose exactly one enrollment flow that
strictly orders invite-token validation, profile staging, and TOTP
finalization. The legacy single-form "stage operator" surface SHALL NOT be
rendered.

#### Scenario: No legacy stage form is rendered
- **WHEN** `<tachyon-iam>` mounts in `auth` mode
- **THEN** the rendered Shadow DOM does not contain an `iam-signup-form`
  element
- **AND** the operator must explicitly press the "Register with Invite
  Token" entry point to begin enrollment

#### Scenario: Enrollment requires the validate-token step
- **GIVEN** the operator opens the enrollment entry point
- **WHEN** they submit a profile or TOTP code
- **THEN** the component refuses to call `stage_signup` until
  `validate_signup_token` has succeeded for the same Mesh Node URL

### Requirement: Login Always Finalizes Through MFA
The `<tachyon-iam>` component SHALL treat every successful login staging as
requiring MFA finalization, matching the backend contract.

#### Scenario: Login transitions to MFA step
- **WHEN** `authn_login` returns a staged login session
- **THEN** the component stores the session id, switches to the MFA step,
  and only emits `iam:authenticated` after `finalize_login` succeeds
- **AND** the dead "no MFA required" branch is never executed

### Requirement: Operator-visible Strings Are Localized
The `<tachyon-iam>` and `<tachyon-mfa-prompt>` components SHALL source every
operator-visible string from `utils/i18n.ts` rather than hardcoding it.

#### Scenario: Language toggle propagates to IAM
- **WHEN** the operator switches the shell language to `fr`
- **THEN** the IAM overlay placeholders, button labels, error messages, and
  toast notifications render in French
- **AND** the step-up MFA prompt uses the same dictionary

### Requirement: Users And Groups Panel
Tachyon-UI SHALL expose a `<tachyon-users-panel>` web component routed
at `/users` that lists every enrolled user, their group memberships,
their roles, their last login timestamp, and their account status, with
inline actions to disable, re-enable, edit groups, edit roles, view
audit history, and delete a user.

#### Scenario: Panel mirrors backend list
- **WHEN** the operator opens the `users` route
- **THEN** the panel calls `iam_list_users` and renders one row per
  user
- **AND** each row shows status, groups, roles, last login, and an
  actions menu

#### Scenario: Inline action invokes the corresponding command
- **WHEN** the operator clicks "Disable" on a row
- **THEN** the panel calls `iam_update_user` with
  `{ disabled: true }`
- **AND** refreshes the table on success

#### Scenario: Audit modal shows the per-user history
- **WHEN** the operator clicks "View audit" on a row
- **THEN** the panel calls `fetch_user_audit_log` for that username
- **AND** displays the entries in a modal with timestamp, action,
  outcome, and detail columns

### Requirement: Group Catalog
The `<tachyon-users-panel>` component SHALL render a group catalog
alongside the user table that lists every group with its description,
roles, scopes, and member count, plus a form to create or update a
group and a control to delete a group.

#### Scenario: Catalog reflects backend state
- **WHEN** the panel loads
- **THEN** it calls `iam_list_groups`
- **AND** renders one card per group with role badges and scope badges

#### Scenario: Form creates and edits the same way
- **WHEN** the operator submits the create form with a name, roles, and
  scopes
- **THEN** the panel calls `iam_upsert_group`
- **AND** subsequently selecting the same group's "Edit" control
  populates the form with the stored values

