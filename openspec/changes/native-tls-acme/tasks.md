# Tasks: Change 057 Implementation

**Agent Instruction:** Implement TLS termination at the edge of the Rust host. TLS handshakes must not block the main Tokio executor threads.

- [ ] Add native TLS termination to the host with dynamic SNI-based certificate resolution for HTTP and Layer 4 listeners.
- [ ] Create a cert-manager system FaaS that provisions ACME certificates and persists them through the storage broker.
- [ ] Suspend cache-miss handshakes asynchronously while the cert-manager provisions or restores certificate material, then resume the TLS flow from cache.
- [ ] Validate first-request ACME issuance and fast cached handshakes for subsequent requests on a real test domain.
