## ADDED Requirements

### Requirement: Signer CLI produces a sealed integrity manifest
The workspace SHALL provide a `cli-signer` binary that generates an Ed25519 key pair, hashes the canonical configuration payload, signs that hash, and writes an `integrity.lock` file at the workspace root containing `config_payload`, `public_key`, and `signature`.

#### Scenario: Generating a fresh integrity manifest
- **WHEN** a developer runs `cargo run -p cli-signer`
- **THEN** the command creates or updates `integrity.lock` in the workspace root
- **AND** the file contains the canonical configuration payload
- **AND** the file contains a hex-encoded public key and signature generated from that payload

### Requirement: Host build embeds integrity material from the manifest
The `core-host` build pipeline SHALL read `../integrity.lock` during compilation, re-run the build when that file changes, and expose the sealed configuration payload, public key, and signature to the binary through compile-time environment variables.

#### Scenario: Rebuilding after the manifest changes
- **WHEN** `core-host` is compiled and `integrity.lock` exists
- **THEN** `build.rs` reads the manifest before Rust compilation finishes
- **AND** Cargo is instructed to re-run the build if `integrity.lock` changes
- **AND** the binary receives `FAAS_CONFIG`, `FAAS_PUBKEY`, and `FAAS_SIGNATURE` values derived from the manifest

### Requirement: Host validates the sealed configuration before serving traffic
At startup, `core-host` SHALL hash the embedded configuration payload, reconstruct the verifying key and signature from the embedded hex values, and refuse to start the HTTP server if signature verification fails.

#### Scenario: Startup succeeds with a valid sealed configuration
- **WHEN** `core-host` starts with embedded integrity values that match the sealed configuration payload
- **THEN** the host verifies the signature successfully
- **AND** the host logs that the integrity check passed
- **AND** the HTTP server continues booting normally

#### Scenario: Startup aborts after configuration tampering
- **WHEN** the embedded signature does not validate the sealed configuration payload
- **THEN** `core-host` aborts startup immediately
- **AND** the process surfaces an integrity validation failure before binding the HTTP server
