## ADDED Requirements

### Requirement: The workspace provides a desktop manifest generator backed by the renamed UI crate
The workspace SHALL provide the manifest generation entrypoint through `tachyon-ui`, preserving the existing Ed25519 signing flow while allowing other local clients to reuse shared read-only status helpers.

#### Scenario: Generating a fresh integrity manifest with tachyon-ui
- **WHEN** a developer runs `cargo run -p tachyon-ui -- generate --route /api/guest-example --memory 64`
- **THEN** the command creates or updates `integrity.lock` in the workspace root
- **AND** the manifest still contains `config_payload`, `public_key`, and `signature`
