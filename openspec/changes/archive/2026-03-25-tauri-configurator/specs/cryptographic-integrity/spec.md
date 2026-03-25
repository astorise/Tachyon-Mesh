## MODIFIED Requirements

### Requirement: Signer CLI produces a sealed integrity manifest
The workspace SHALL provide a `tachyon-cli` manifest generator, backed by a Tauri application configured for CLI use, that generates an Ed25519 key pair, hashes the canonical configuration payload, signs that hash, and writes an `integrity.lock` file at the workspace root containing `config_payload`, `public_key`, and `signature`.

#### Scenario: Generating a fresh integrity manifest with tachyon-cli
- **WHEN** a developer runs `cargo run -p tachyon-cli -- generate --route /api/guest-example --memory 64`
- **THEN** the command creates or updates `integrity.lock` in the workspace root
- **AND** the file contains the canonical configuration payload derived from the supplied CLI options
- **AND** the file contains a hex-encoded public key and signature generated from that payload
