## ADDED Requirements

### Requirement: system-faas-tde performs AES-256-GCM block encryption
The Mesh SHALL provide `system-faas-tde`, a dedicated module responsible exclusively for AES-256-GCM encryption and decryption of fixed-size blocks supplied via zero-trust IPC.

#### Scenario: Block round-trips through TDE FaaS
- **WHEN** the host sends a plaintext block to `system-faas-tde` for encryption
- **THEN** the FaaS returns a ciphertext block authenticated under AES-256-GCM
- **WHEN** the host later sends that ciphertext back for decryption
- **THEN** the FaaS returns the original plaintext block
- **AND** the FaaS rejects ciphertext that fails AEAD authentication

### Requirement: integrity.lock allows flagging volume mounts as encrypted
The `integrity.lock` manifest SHALL allow individual volume mounts to be flagged with `encrypted: true`. Volumes without this flag SHALL bypass the TDE FaaS entirely.

#### Scenario: Encrypted volume routes I/O through TDE FaaS
- **WHEN** a Wasm module writes to a volume mount flagged `encrypted: true`
- **THEN** the host intercepts the WASI file descriptor write call
- **AND** routes the byte stream through `system-faas-tde` over zero-trust IPC before persisting the resulting ciphertext to disk
- **WHEN** the same module reads from that volume
- **THEN** the host fetches the ciphertext from disk, routes it through `system-faas-tde` for decryption, and returns the plaintext to the guest

#### Scenario: Unencrypted volume retains native disk speed
- **WHEN** a Wasm module writes to a volume mount that does not set `encrypted: true`
- **THEN** the host writes directly to disk without invoking `system-faas-tde`
- **AND** the I/O latency matches the pre-TDE baseline
