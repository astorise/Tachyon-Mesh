## ADDED Requirements

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
