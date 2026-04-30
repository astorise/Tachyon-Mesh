# device-flow-enrollment Specification

## Purpose
TBD - created by archiving change device-flow-enrollment-faas. Update Purpose after archive.
## Requirements
### Requirement: Unenrolled nodes start an outbound enrollment FaaS with a short PIN
On first boot, when no valid `integrity.lock` or mTLS certificate is present, the `core-host` SHALL instantiate `system-faas-enrollment`, which generates a short, human-readable PIN and opens an outbound HTTP/3 or WebSocket tunnel to a known cluster endpoint while transmitting the new node's public key.

#### Scenario: Fresh node initiates enrollment from behind NAT
- **WHEN** a Tachyon node boots without a valid `integrity.lock` or mTLS credentials
- **THEN** `core-host` instantiates `system-faas-enrollment`
- **AND** the FaaS generates a short PIN (for example `A7X-92B`)
- **AND** the FaaS establishes an outbound tunnel to a known cluster endpoint
- **AND** the FaaS publishes the node's public key over that tunnel and waits for an approval

### Requirement: Operator approval over an active node injects credentials through the open tunnel
An administrator SHALL be able to approve a pending enrollment by entering the PIN through Tachyon Studio while connected to any active node in the masterless mesh; the active node SHALL sign the new node's public key and deliver the certificate back through the existing outbound tunnel.

#### Scenario: PIN approval completes enrollment
- **WHEN** an administrator enters the PIN in Tachyon Studio while connected to any active mesh node
- **THEN** the active node signs the pending node's public key with the cluster CA
- **AND** sends the signed certificate down the open enrollment tunnel
- **AND** `system-faas-enrollment` persists the credentials, terminates itself, and triggers `core-host` to load the full mesh-overlay configuration

### Requirement: Outbound device enrollment FaaS
Nodes without valid cluster credentials MUST enter bootstrap mode and run only the enrollment System FaaS until enrollment completes.

#### Scenario: Clean node starts without credentials
- **GIVEN** a node starts without a valid cluster certificate or integrity lock
- **WHEN** the host finishes its credential check
- **THEN** it enters bootstrap mode
- **AND** it loads `system-faas-enrollment` as the only executable workload
- **AND** inbound management and mesh traffic remain disabled until enrollment succeeds

### Requirement: PIN-based outbound approval
The enrollment FaaS MUST generate a short-lived PIN, establish an outbound tunnel to a bootstrap endpoint, and use that tunnel to receive signed credentials.

#### Scenario: Administrator approves a pending node
- **GIVEN** the enrollment FaaS has opened an outbound enrollment tunnel and displayed a PIN
- **WHEN** an administrator submits the PIN to an active node
- **THEN** the active node signs the pending node public key
- **AND** the certificate is delivered back over the existing outbound tunnel
- **AND** the enrolling node stores the certificate and exits bootstrap mode

### Requirement: Enrollment handoff
The host MUST transition from bootstrap mode to normal mesh operation after the enrollment FaaS persists credentials.

#### Scenario: Enrollment completes successfully
- **GIVEN** the enrollment FaaS has stored valid cluster credentials
- **WHEN** it signals `ENROLLMENT_COMPLETE` to the host
- **THEN** the host stops the enrollment FaaS
- **AND** it loads the normal mesh overlay and configured workloads
- **AND** subsequent restarts skip bootstrap mode while credentials remain valid

