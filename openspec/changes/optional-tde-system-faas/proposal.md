# Proposal: Optional TDE via System FaaS

## Context
In Edge Computing, physical security is a major concern. If an unauthorized actor gains physical access to a Tachyon Mesh node, they could extract the persistent storage drive and read sensitive data (e.g., AI models, patient records) in plaintext. While Transparent Data Encryption (TDE) solves this, enforcing it globally at the `core-host` level introduces a massive CPU and latency overhead for all standard I/O operations.

## Proposed Solution
We will implement TDE as an **opt-in feature powered by a dedicated System FaaS**:
1. **Modularity:** Create a new `system-faas-tde` module responsible exclusively for AES-256-GCM block encryption and decryption.
2. **Opt-In Configuration:** The `integrity.lock` manifest will allow developers to flag specific volume mounts with `encrypted: true`.
3. **WASI Interception:** When a FaaS module writes to an encrypted volume, the `core-host` will seamlessly intercept the WASI (WebAssembly System Interface) file descriptor calls and route the byte stream through `system-faas-tde` via zero-trust IPC before hitting the physical disk. Unencrypted volumes will bypass this FaaS entirely, maintaining native disk speeds.

## Objectives
- Provide physical data security for sensitive Edge deployments.
- Ensure zero CPU overhead for non-sensitive I/O operations.
- Decouple encryption logic from the `core-host`, allowing keys and algorithms to be managed and rotated via the FaaS lifecycle.