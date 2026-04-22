## MODIFIED Requirements

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
