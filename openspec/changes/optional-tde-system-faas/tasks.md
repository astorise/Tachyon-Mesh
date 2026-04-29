# Implementation Tasks

## Phase 1: Configuration & Schema
- [x] Update the `VolumeMount` struct in `core-host` to parse the `encrypted: bool` property.
- [ ] Update the CLI/Tauri Configurator schemas to support the `encrypted` toggle in the UI.

## Phase 2: TDE System FaaS
- [x] Bootstrap the `systems/system-faas-tde` crate.
- [x] Implement AES-256-GCM encryption/decryption using a lightweight Rust crypto crate (like `ring` or `aes-gcm`).
- [x] Expose the encrypt/decrypt functions via the standard Tachyon IPC interface.

## Phase 3: Host Integration (WASI Interceptor)
- [x] In `core-host`, locate the WASI filesystem initialization logic.
- [x] Implement a branching logic: native mount vs. encrypted virtual mount.
- [ ] Build the WASI VFS wrapper that proxies chunked reads/writes to the `system-faas-tde` IPC endpoints.

## Phase 4: Validation
- [ ] **Performance Test:** Write a 1GB file to a standard volume and measure the time. Write a 1GB file to an encrypted volume and measure the time. Ensure the standard volume time is unaffected by the new feature.
- [ ] **Security Test:** Write plaintext data to an encrypted volume. Read the physical host directory from the outside OS (e.g., using `cat`) and verify the data is unreadable cipher text.
