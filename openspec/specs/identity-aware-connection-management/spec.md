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
The desktop client SHALL hold a runtime connection profile with the node URL, bearer token, and optional mTLS identity bytes.

#### Scenario: A new node connection is established
- **WHEN** the user submits a node URL, admin token, and optional identity bundle
- **THEN** the client stores the profile in a global `RwLock`
- **AND** it validates the profile by calling `/admin/status` before reporting success

### Requirement: The UI blocks dashboard access until a node connection succeeds
The desktop UI SHALL display a full-screen connection overlay until a node connection has been authenticated successfully.

#### Scenario: The user connects successfully
- **WHEN** `connect_to_node` returns successfully
- **THEN** the overlay fades out
- **AND** the dashboard refresh button reads status through the authenticated client layer

