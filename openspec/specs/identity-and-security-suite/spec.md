# identity-and-security-suite Specification

## Purpose
TBD - created by archiving change identity-and-security-suite. Update Purpose after archive.
## Requirements
### Requirement: Recovery codes are generated once and stored only as hashes
The AuthN component SHALL generate recovery codes in plaintext once, persist only hashed values, and associate them with the requesting username.

#### Scenario: An operator initializes recovery codes
- **WHEN** the desktop client requests recovery codes for the administrator
- **THEN** the AuthN component generates ten codes in the `TCHN-XXXXX-XXXXX` format
- **AND** it stores only their hashes in persistent auth state
- **AND** it returns the plaintext codes exactly once in the response

### Requirement: Recovery codes can mint a short-lived emergency session
The AuthN component SHALL burn a recovery code immediately when it is redeemed and return a short-lived JWT for emergency recovery.

#### Scenario: A valid recovery code is redeemed
- **WHEN** a caller submits a stored recovery code for a username
- **THEN** the matching hash is removed from storage
- **AND** the AuthN component returns a temporary authenticated JWT

#### Scenario: An invalid or reused recovery code is redeemed
- **WHEN** the submitted recovery code does not match any stored hash
- **THEN** the request is rejected
- **AND** no new JWT is issued

### Requirement: Personal access tokens are issued once and stored only as hashes
The AuthN component SHALL mint Personal Access Tokens in plaintext once, persist only hashed token material, and bind each token to the owning subject plus the requested scopes and expiry.

#### Scenario: An operator issues a PAT
- **WHEN** an authenticated administrator requests a new PAT with a name, scope list, and TTL
- **THEN** the AuthN component generates a random plaintext PAT
- **AND** it stores only the token hash with the name, scopes, owner, and expiry metadata
- **AND** it returns the plaintext token exactly once in the response

#### Scenario: An expired PAT is presented
- **WHEN** a caller presents a PAT after its expiry time
- **THEN** AuthN rejects the token
- **AND** the host responds with `401 Unauthorized`

### Requirement: The desktop UI exposes a personal account security view
The desktop UI SHALL expose a dedicated `My Account` view where the currently connected operator can issue PATs and trigger personal security actions without reusing the shared identity-management panel.

#### Scenario: The operator opens My Account
- **WHEN** the operator selects the `My Account` sidebar entry
- **THEN** the desktop frontend switches to `#view-account`
- **AND** it renders PAT issuance controls and personal security actions for the active administrative connection

#### Scenario: The operator generates a PAT from the desktop UI
- **WHEN** the operator submits a PAT name from the `My Account` view
- **THEN** the frontend invokes the shared Tauri command for PAT issuance
- **AND** the returned token is rendered once in the account view

### Requirement: First-run onboarding forces the operator to save recovery codes
The desktop UI SHALL block completion of the first-run security modal until recovery codes are displayed and explicitly saved.

#### Scenario: Recovery codes are generated during onboarding
- **WHEN** the operator advances from the QR step to the recovery-code step
- **THEN** the UI requests recovery codes through the shared client layer
- **AND** it renders the returned codes in the modal
- **AND** it blocks completion until the operator downloads the text bundle

### Requirement: The desktop UI exposes a routed IAM dashboard for the active admin session
The desktop UI SHALL expose a dedicated `Identity` view that renders the active administrative subject, groups, endpoint posture, and supported security actions without leaving the authenticated shell.

#### Scenario: The operator opens Identity after authentication
- **WHEN** the operator selects the `Identity` sidebar entry after the AuthN gateway is dismissed
- **THEN** the desktop frontend switches to `#view-identity`
- **AND** it renders an IAM summary table for the active administrative session
- **AND** it shows the connected endpoint plus the current MFA and recovery-bundle posture

### Requirement: The IAM dashboard reuses existing security operations
The Identity view SHALL wire its security actions to the existing recovery-bundle APIs instead of inventing unsupported remote IAM CRUD operations.

#### Scenario: The operator regenerates the recovery bundle from Identity
- **WHEN** the operator triggers the IAM dashboard action to rotate recovery material for the active subject
- **THEN** the frontend invokes the shared Tauri/client command for recovery regeneration
- **AND** it renders the returned bundle in the Identity view
- **AND** it preserves the dedicated `My Account` PAT workflow for personal token issuance

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

### Requirement: The desktop UI exposes a Resource Catalog view
The desktop UI SHALL register a `Resource Catalog` sidebar entry that switches to a `#view-resources` container rendering all logical mesh resources, distinguishing internal IPC aliases from external HTTPS egress targets and tagging entries that have not yet been re-sealed into `integrity.lock`.

#### Scenario: The operator opens the Resource Catalog
- **WHEN** the operator selects the `Resource Catalog` sidebar entry
- **THEN** the desktop frontend switches to `#view-resources`
- **AND** the view invokes the `get_resources` Tauri command
- **AND** rows are rendered with a colored badge per type (cyan = internal, blue = external, amber = pending seal)

#### Scenario: Pending overlay entries are visually distinguished from sealed entries
- **WHEN** an entry exists in the workspace overlay file but not in the sealed `integrity.lock`
- **THEN** the row is rendered with the amber `pending seal` badge
- **AND** the row exposes a tooltip indicating that a CLI re-seal is required to promote the resource

### Requirement: Tauri commands read sealed and pending mesh resources
The desktop backend SHALL register a `get_resources` Tauri command that returns the union of resources sealed in `integrity.lock` (via `tachyon_client::read_lockfile`) and resources staged in the workspace overlay file `tachyon.resources.json`. Overlay entries SHALL be flagged with `pending: true` so the UI can render the pending badge.

#### Scenario: get_resources merges sealed and overlay resources
- **WHEN** the desktop frontend invokes the `get_resources` Tauri command
- **THEN** the backend reads sealed resources from the workspace `integrity.lock`
- **AND** it merges entries from `tachyon.resources.json` with `pending: true`
- **AND** it returns a single deduplicated list keyed by resource name

### Requirement: Tauri commands stage mesh resources without re-sealing the lockfile
The desktop backend SHALL register `save_resource` and `delete_resource` Tauri commands that write to the workspace overlay file `tachyon.resources.json` rather than mutating `integrity.lock`. `save_resource` SHALL reject inputs whose name is empty and SHALL reject `external` resources whose target is not an HTTPS URL or a recognised loopback / `*.svc` cluster-local hostname. `delete_resource` SHALL succeed only when the resource exists in the overlay; resources that exist only in the sealed lockfile SHALL return an error directing the operator to perform a CLI re-seal.

#### Scenario: Saving an external resource validates the target
- **WHEN** the desktop frontend invokes `save_resource` with `type: "external"` and `target: "ftp://example.com"`
- **THEN** the backend rejects the request with a validation error
- **AND** the overlay file is left unchanged

#### Scenario: Saving a valid external resource writes to the overlay
- **WHEN** the desktop frontend invokes `save_resource` with `name: "stripe-api"`, `type: "external"`, `target: "https://api.stripe.com"`
- **THEN** the backend appends the entry to `tachyon.resources.json`
- **AND** subsequent `get_resources` calls include the resource flagged `pending: true`

#### Scenario: Deleting a sealed-only resource surfaces the re-seal hint
- **WHEN** the desktop frontend invokes `delete_resource` for a name that exists in `integrity.lock` but not in the overlay
- **THEN** the backend returns an error mentioning that a CLI re-seal is required
- **AND** the overlay file is left unchanged

