# Native TLS ACME

## Purpose
Define how Tachyon terminates TLS natively at the host edge, maps custom domains through SNI, and provisions certificates asynchronously through a cert-manager workflow.

## Requirements
### Requirement: Targets can declare custom domains for native TLS termination
The integrity manifest SHALL allow targets to declare one or more custom domains that the host can terminate natively with SNI-aware certificate selection.

#### Scenario: A target declares custom domains
- **WHEN** an operator configures one or more domains for a target
- **THEN** the host associates those domains with the target for HTTPS or TLS-wrapped Layer 4 routing

### Requirement: TLS handshakes resolve certificates dynamically from cache or cert manager
The host SHALL inspect the incoming SNI value during the TLS handshake, serve cached certificates immediately when available, and asynchronously obtain missing certificates through the cert manager without aborting the connection.

#### Scenario: A cached certificate exists for the requested SNI
- **WHEN** a client starts a TLS handshake for a known domain with a cached certificate
- **THEN** the host completes the handshake locally
- **AND** forwards the decrypted stream to the configured router

#### Scenario: No cached certificate exists for the requested SNI
- **WHEN** a client starts a TLS handshake for a known domain with no cached certificate
- **THEN** the host suspends only that handshake flow
- **AND** invokes the cert manager to provision or load the certificate
- **AND** resumes the handshake after the certificate is cached

### Requirement: The cert manager persists provisioned certificates through the storage broker
The cert manager SHALL obtain ACME certificates, satisfy the certificate challenge flow, persist the certificate material through the storage broker, and return the resulting keypair to the host cache.

#### Scenario: A new certificate is issued through the cert manager
- **WHEN** the cert manager successfully provisions certificate material for a target domain
- **THEN** it stores the private key and certificate chain in persistent storage through the storage broker
- **AND** returns the certificate material to the host so future handshakes can complete from cache
