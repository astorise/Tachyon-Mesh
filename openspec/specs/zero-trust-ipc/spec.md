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

#### Scenario: A guest attempts to spoof the identity header
- **WHEN** a guest issues an outbound mesh request with its own `X-Tachyon-Identity` header
- **THEN** the host removes the spoofed value
- **AND** injects a new signed identity token for the actual caller

### Requirement: Storage broker authorization is enforced from the signed identity
The storage broker SHALL verify the signed identity token, determine the caller target from that token, and reject write attempts that exceed the caller's allowed volume scope.

#### Scenario: A caller writes outside its allowed volume scope
- **WHEN** the storage broker receives a request with a valid signed identity token
- **AND** the requested path is outside the caller's authorized volume mapping
- **THEN** the broker returns HTTP 403 and denies the write

