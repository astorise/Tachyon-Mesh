# identity-aware-connection-management Specification

## Purpose
TBD - created by archiving change identity-aware-connection-management. Update Purpose after archive.
## Requirements
### Requirement: Administrative connections use a dedicated auth component
The mesh SHALL expose a dedicated `tachyon:identity/auth` WebAssembly component that validates administrative bearer tokens before admin routes are served.

#### Scenario: A valid admin token reaches an admin route
- **WHEN** the host receives `Authorization: Bearer <token>` on an `/admin/*` route
- **THEN** it invokes `system-faas-auth`
- **AND** it allows the request only when the token is valid and includes the `admin` role

#### Scenario: An invalid token reaches an admin route
- **WHEN** the bearer token is missing, malformed, expired, or signed with the wrong secret
- **THEN** the host rejects the request with `401 Unauthorized`

#### Scenario: A token lacks the required role
- **WHEN** the token validates cryptographically but does not include the required `admin` role
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

