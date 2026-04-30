# mtls-encryption Specification

## Purpose
Define TLS, mTLS, and FIPS-compatible cryptographic behavior for external and inter-node communication.

## Requirements
### Requirement: Strict TLS 1.3 communication
The host SHALL secure configured external and inter-node channels with strict TLS settings.

#### Scenario: Client presents invalid TLS parameters
- **WHEN** a client attempts a legacy or non-compliant handshake
- **THEN** the host rejects the connection before forwarding traffic to a guest

### Requirement: FIPS-capable cryptographic backend
The host SHALL support a FIPS feature mode backed by the approved rustls provider configuration.

#### Scenario: FIPS mode is enabled
- **WHEN** the host is compiled with the FIPS feature
- **THEN** it selects the FIPS-capable backend and rejects cipher suites outside the approved profile
