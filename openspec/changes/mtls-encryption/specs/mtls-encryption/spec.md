## ADDED Requirements

### Requirement: A system gateway can delegate mTLS handshakes to host cryptography capabilities
The platform SHALL expose host-managed mTLS through a dedicated capability so a gateway function can accept raw sockets and forward decrypted traffic into the mesh.

#### Scenario: A gateway upgrades an inbound socket with mTLS
- **WHEN** a privileged gateway receives a raw TCP connection that requires mutual TLS
- **THEN** it asks the host cryptography capability to complete the handshake
- **AND** forwards the resulting decrypted byte stream to the internal routing path
