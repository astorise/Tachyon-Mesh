## ADDED Requirements

### Requirement: The desktop signin view rejects bootstrap tokens
The desktop UI SHALL keep the signin step (`#auth-step-login`) limited to a node URL, username, password, and optional mTLS client bundle, with no input that accepts a one-time invite or bootstrap token. Token-based onboarding SHALL be reachable only through the dedicated `Register with Invite Token` action that switches the auth flow to `#auth-step-signup-token`.

#### Scenario: An operator opens the signin step
- **WHEN** the desktop UI activates `#auth-step-login`
- **THEN** the rendered form contains exactly the URL, username, password and mTLS-bundle controls
- **AND** no input accepts a one-time signup or bootstrap token
- **AND** a secondary action labelled `Register with Invite Token` switches the flow to `#auth-step-signup-token`

#### Scenario: An operator presses the password visibility toggle on signin
- **WHEN** the operator clicks the eye icon embedded in the signin password field
- **THEN** the input `type` attribute toggles between `password` and `text`
- **AND** no other field on the signin step changes its visibility

### Requirement: Signup usernames are deterministically derived from the operator's name
The signup profile step (`#auth-step-signup-profile`) SHALL compute the `username` field from `firstName` and `lastName` using a `formatUsername(first, last)` helper that lowercases the inputs, removes diacritics via NFD normalization, replaces runs of whitespace with a single dot, and strips characters outside `[a-z0-9._-]`. The username input SHALL be marked `readonly` so the operator cannot diverge from the computed identity.

#### Scenario: Diacritics are stripped from the computed username
- **WHEN** the operator types `Sébastien` into the first-name field and `Astori` into the last-name field
- **THEN** the username field renders the value `sebastien.astori`
- **AND** the username field is marked `readonly` and cannot be edited directly

#### Scenario: Compound names collapse whitespace to a single dot
- **WHEN** the operator types `Marie Claire` and `Du Pont`
- **THEN** the computed username is `marie.claire.du.pont`
- **AND** characters outside `[a-z0-9._-]` are removed before the value is staged

### Requirement: Signup enforces password confirmation before staging
The signup profile step SHALL render a `confirmPassword` input alongside `password` and SHALL gate the `Next` action until both inputs are non-empty, equal, and at least 8 characters long. The confirmation value SHALL NOT be sent to the backend `stage_signup` command. A mismatch SHALL render an inline error in red text below the confirmation field.

#### Scenario: Mismatched passwords block staging
- **WHEN** the operator enters `correct horse battery staple` in `password` and `correct horse battery stapl3` in `confirmPassword`
- **THEN** the `Next` button stays disabled
- **AND** an inline error message in red is rendered beneath the confirmation field
- **AND** the desktop UI does not invoke the `stage_signup` Tauri command

#### Scenario: Matching passwords unlock staging without leaking the confirmation
- **WHEN** the operator enters identical 12-character passwords in both fields
- **THEN** the `Next` button becomes enabled
- **AND** invoking `Next` calls `stage_signup` with the `password` field only
- **AND** the request payload does not include any `confirmPassword` property

### Requirement: Password visibility is toggleable on every password input
The desktop UI SHALL render an inline eye icon button on every password input (`#auth-password`, `#signup-password`, `#signup-confirm-password`) that toggles the input's `type` between `password` and `text` independently per field, without revealing other fields.

#### Scenario: Toggling one password input does not reveal another
- **WHEN** the operator toggles the eye icon on `#signup-password`
- **THEN** only the `#signup-password` input switches `type` to `text`
- **AND** `#signup-confirm-password` and `#auth-password` remain `type="password"`
