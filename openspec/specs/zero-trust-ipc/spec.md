# zero-trust-ipc Specification

## Purpose
TBD - created by archiving change zero-trust-ipc. Update Purpose after archive.
## Requirements
### Requirement: The host generates an ephemeral signing identity for system IPC
The host SHALL generate an in-memory Ed25519 keypair at startup and expose the public verification key to system FaaS components that need to authenticate mesh requests.

#### Scenario: A system FaaS starts under the host
- **WHEN** the host instantiates a system FaaS
- **THEN** the system FaaS receives the host public key through a trusted runtime channel such as an environment variable or host capability

### Requirement: Outbound mesh requests always carry host-signed identity headers
The host SHALL strip any user-supplied identity header from outbound mesh traffic and replace it with a short-lived host-signed identity token that describes the calling target.

The host SHALL memoize route identity tokens by normalized route path and route role while the token remains valid, refreshing cached entries before the TTL window expires and clearing the route-token cache after a successful manifest reload.

#### Scenario: A guest attempts to spoof the identity header
- **WHEN** a guest issues an outbound mesh request with its own `X-Tachyon-Identity` header
- **THEN** the host removes the spoofed value
- **AND** injects a new signed identity token for the actual caller

#### Scenario: Reuses identity token within the TTL window
- **GIVEN** a route has already issued an internal mesh request
- **WHEN** the same route issues another internal mesh request before the identity token refresh window
- **THEN** the host reuses the cached signed identity token for that route and role
- **AND** a different route or role receives a distinct cached token

#### Scenario: Clears cached identity tokens on manifest reload
- **GIVEN** route identity tokens are cached
- **WHEN** the host successfully reloads the manifest
- **THEN** the cached route identity tokens are cleared before new mesh dispatches use the updated runtime configuration

### Requirement: Storage broker authorization is enforced from the signed identity
The storage broker SHALL verify the signed identity token, determine the caller target from that token, and reject write attempts that exceed the caller's allowed volume scope.

#### Scenario: A caller writes outside its allowed volume scope
- **WHEN** the storage broker receives a request with a valid signed identity token
- **AND** the requested path is outside the caller's authorized volume mapping
- **THEN** the broker returns HTTP 403 and denies the write
