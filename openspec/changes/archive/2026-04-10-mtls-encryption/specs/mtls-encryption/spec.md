## ADDED Requirements

### Requirement: The host can terminate mutually authenticated TLS for a sealed gateway route
The platform SHALL expose an opt-in host-managed mTLS listener that requires valid client certificates and only starts when the sealed manifest includes the dedicated `/system/gateway` system route.

#### Scenario: The sealed manifest enables the gateway route
- **WHEN** the operator seals `/system/gateway` into the manifest and provides the server certificate, private key, and client CA material
- **THEN** the host starts the mTLS listener
- **AND** only accepts requests from clients that present a certificate trusted by the configured CA

### Requirement: Authorized mTLS traffic is forwarded to the mesh through the system gateway
The platform SHALL preserve the original request path from an authorized mTLS connection, dispatch the request to `/system/gateway`, and let the gateway forward the request internally through outbound HTTP.

#### Scenario: A trusted client reaches the mesh through the gateway
- **WHEN** an mTLS request is accepted by the host
- **THEN** the host injects the original request path into `x-tachyon-original-route`
- **AND** the gateway forwards the request to the target mesh route
