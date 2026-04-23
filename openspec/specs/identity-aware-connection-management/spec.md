# identity-aware-connection-management Specification

## Purpose
TBD - created by archiving change identity-aware-connection-management. Update Purpose after archive.
## Requirements
### Requirement: Administrative connections use dedicated AuthN and AuthZ components
The mesh SHALL expose dedicated `tachyon:identity/authn` and `tachyon:identity/authz` WebAssembly components that validate administrative bearer credentials and then authorize the requested admin action before `/admin/*` routes are served.

#### Scenario: A valid administrative JWT reaches an admin route
- **WHEN** the host receives `Authorization: Bearer <token>` on an `/admin/*` route
- **THEN** it invokes `system-faas-authn`
- **AND** AuthN returns an identity payload containing the authenticated subject, roles, and scopes
- **AND** the host invokes `system-faas-authz` with that identity plus the HTTP method and request path
- **AND** the request is allowed only when AuthZ returns `true`

#### Scenario: A valid scoped PAT reaches an admin route
- **WHEN** the host receives a valid PAT on an `/admin/*` route
- **THEN** AuthN resolves the PAT from hashed persisted state
- **AND** AuthN returns the subject and PAT scopes attached to that token
- **AND** AuthZ allows only the admin resources mapped to those scopes

#### Scenario: Authentication fails before policy evaluation
- **WHEN** the bearer token is missing, malformed, expired, or unknown
- **THEN** the host rejects the request with `401 Unauthorized`
- **AND** AuthZ is not invoked

#### Scenario: Authorization denies the authenticated caller
- **WHEN** AuthN succeeds but the identity lacks the role or scope required for the requested admin route
- **THEN** the host rejects the request with `403 Forbidden`

### Requirement: The desktop client keeps an authenticated connection profile
The desktop client SHALL hold a runtime connection profile with the node URL, operator name, bearer token, and optional mTLS identity bytes used by the AuthN gateway.

#### Scenario: The operator submits the AuthN gateway form
- **WHEN** the operator enters a mesh URL, username, bootstrap secret or admin token, and an optional mTLS identity bundle
- **THEN** the frontend validates the connection through the existing client/Tauri bridge before unlocking the session
- **AND** it stores the resulting authenticated connection profile for subsequent admin requests
- **AND** it retains the submitted operator name for IAM rendering in the desktop shell

### Requirement: The UI blocks dashboard access until a node connection succeeds
The desktop UI SHALL display a full-screen authentication gateway until the operator completes the staged login and MFA flow successfully.

#### Scenario: The operator advances from login to MFA
- **WHEN** the login step succeeds
- **THEN** the frontend animates the AuthN gateway from the credential form to the MFA form
- **AND** the dashboard remains blocked while the MFA step is pending

#### Scenario: The operator completes MFA
- **WHEN** the operator submits a valid-looking MFA code from the AuthN gateway
- **THEN** the frontend fades out the overlay with GSAP
- **AND** it unlocks dashboard navigation for the active authenticated session

