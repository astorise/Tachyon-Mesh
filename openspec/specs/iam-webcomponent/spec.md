# iam-webcomponent Specification

## Purpose
TBD - created by archiving change tachyon-ui-webcomponents-phase1. Update Purpose after archive.
## Requirements
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

