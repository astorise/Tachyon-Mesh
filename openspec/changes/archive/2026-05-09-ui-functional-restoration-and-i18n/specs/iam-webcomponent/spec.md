# iam-webcomponent

## ADDED Requirements

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

