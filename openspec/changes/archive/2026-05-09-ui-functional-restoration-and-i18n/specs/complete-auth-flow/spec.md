# complete-auth-flow

## MODIFIED Requirements

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
