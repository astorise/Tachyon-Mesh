## ADDED Requirements

### Requirement: Tauri configurator supports headless manifest generation from the desktop wrapper
The `tachyon-ui` application SHALL expose the existing `generate` subcommand for manifest sealing while also serving as the desktop wrapper for the Tauri frontend.

#### Scenario: Desktop wrapper still supports manifest generation
- **WHEN** a developer runs `cargo run -p tachyon-ui -- generate --route /api/guest-example --memory 64`
- **THEN** the command writes `integrity.lock` in the workspace root
- **AND** the command succeeds without opening a desktop window

### Requirement: The desktop frontend can invoke shared client status queries
The `tachyon-ui` Rust backend SHALL delegate status queries to the shared `tachyon-client` library instead of embedding duplicated lockfile reading logic in the Tauri wrapper.

#### Scenario: The frontend requests the engine status
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the Tauri command awaits `tachyon_client::get_engine_status()`
- **AND** the returned payload comes from the shared client layer
