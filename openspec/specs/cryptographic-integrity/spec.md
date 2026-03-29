# cryptographic-integrity Specification

## Purpose
Define how the workspace seals an integrity manifest and how `core-host` verifies that embedded configuration before serving traffic.
## Requirements
### Requirement: Signer CLI produces a sealed integrity manifest
The workspace SHALL provide a `tachyon-cli` manifest generator, backed by a Tauri application configured for CLI use, that generates an Ed25519 key pair, hashes the canonical configuration payload, signs that hash, and writes an `integrity.lock` file at the workspace root containing `config_payload`, `public_key`, and `signature`.

#### Scenario: Generating a fresh integrity manifest with tachyon-cli
- **WHEN** a developer runs `cargo run -p tachyon-cli -- generate --route /api/guest-example --memory 64`
- **THEN** the command creates or updates `integrity.lock` in the workspace root
- **AND** the file contains the canonical configuration payload derived from the supplied CLI options
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

### Requirement: Integrity manifest seals route execution roles
The workspace SHALL seal each configured route in `integrity.lock` as a structured entry
containing the normalized route path, an execution role of `user` or `system`, optional
`allowed_secrets`, and route-scaling fields `min_instances` plus `max_concurrency`.

#### Scenario: Generating a manifest with explicit route scaling
- **WHEN** a developer runs `tachyon-cli generate --route /api/guest-example --route-scale /api/guest-example=2:16 --memory 64`
- **THEN** the canonical configuration payload includes `/api/guest-example` with `min_instances = 2`
- **AND** the same route entry includes `max_concurrency = 16`
- **AND** the route remains normalized before it is signed

#### Scenario: Loading an older manifest without scaling fields
- **WHEN** `core-host` starts with a sealed manifest whose route entries omit `min_instances` and `max_concurrency`
- **THEN** integrity validation still succeeds
- **AND** the host defaults `min_instances` to `0`
- **AND** the host defaults `max_concurrency` to `100`

### Requirement: Host rejects ambiguous or invalid sealed route metadata
`core-host` SHALL normalize sealed route paths, reject duplicates, and refuse to start if any sealed route metadata is invalid.

#### Scenario: Startup aborts after duplicate route metadata
- **WHEN** the embedded configuration payload contains the same normalized route more than once
- **THEN** `core-host` fails integrity validation before serving traffic
- **AND** the error reports that the sealed route metadata is ambiguous

