# Proposal: Change 048 - Zero-Trust IPC & Cryptographic Identity

## Context
Tachyon relies on internal IPC over the Mesh (UDS or mTLS) for System FaaS operations (e.g., `system-faas-storage-broker` handling file writes). In a multi-tenant PaaS environment, we cannot assume that internal traffic is inherently trustworthy. A malicious User FaaS could forge an HTTP request to the Storage Broker to overwrite another tenant's files. We must implement a Zero-Trust architecture using cryptographic identity injection (similar to SPIFFE/SPIRE).

## Objective
1. The `core-host` must act as the Identity Provider.
2. Every outbound IPC request originating from a WASM FaaS must be transparently intercepted by the host and injected with a cryptographically signed Identity Token (JWT).
3. System FaaS must validate this token and enforce Access Control Lists (ACLs) based on the FaaS identity and the `integrity.lock` definitions before executing sensitive actions.

## Scope
- Generate a fast, ephemeral Ed25519 keypair in the `core-host` at startup.
- Update the Mesh Client to sign outbound requests with `X-Tachyon-Identity`.
- Update `system-faas-storage-broker` to verify signatures and enforce path isolation.

## Success Metrics
- A User FaaS attempting to write to `/data/tenant-b/` via the Storage Broker receives a 403 Forbidden.
- A User FaaS attempting to forge its own `X-Tachyon-Identity` header fails, as the host either strips it or the signature validation fails at the Broker level.