# mTLS Encryption

## Purpose
Define how Tachyon accepts mutually authenticated TLS traffic at the host edge and forwards authorized requests into the mesh through a dedicated system gateway.

## Requirements
### Requirement: The host can terminate mutually authenticated TLS for a sealed gateway route
The host SHALL expose an opt-in mTLS listener that requires valid client certificates and only starts when the sealed manifest includes the dedicated `/system/gateway` system route.

#### Scenario: The sealed manifest enables the gateway route
- **WHEN** the operator seals `/system/gateway` into the manifest and provides the server certificate, private key, and client CA material
- **THEN** the host starts the mTLS listener
- **AND** only accepts requests from clients that present a certificate trusted by the configured CA

#### Scenario: The sealed manifest does not enable the gateway route
- **WHEN** the operator does not seal `/system/gateway`
- **THEN** the host does not bind the mTLS listener
- **AND** incurs no gateway forwarding overhead

### Requirement: Authorized mTLS traffic is forwarded to the mesh through the system gateway
The platform SHALL preserve the original request path from an authorized mTLS connection, dispatch the request to `/system/gateway`, and let the gateway forward the request internally through outbound HTTP.

#### Scenario: A client presents a trusted certificate
- **WHEN** an mTLS request is accepted by the host
- **THEN** the host injects the original request path into `x-tachyon-original-route`
- **AND** dispatches the request to the sealed `/system/gateway` system route
- **AND** the gateway forwards the request to the target mesh route

#### Scenario: The gateway receives an invalid original route
- **WHEN** `/system/gateway` receives an empty route or a self-referential `/system/gateway` route
- **THEN** the gateway rejects the request
- **AND** does not forward it into the mesh

### Requirement: Gateway artifacts are validated with mock certificates
The mTLS gateway workflow SHALL be testable without public infrastructure by using generated certificate material and an in-process authorized client.

#### Scenario: A client omits a certificate
- **WHEN** a client opens an mTLS connection without presenting a trusted certificate
- **THEN** the TLS handshake fails before the request reaches the gateway

#### Scenario: A trusted client reaches a sealed route
- **WHEN** a client opens an mTLS connection with a trusted certificate
- **THEN** the request completes successfully through `/system/gateway`
- **AND** the target route receives the forwarded request body and method semantics intact
