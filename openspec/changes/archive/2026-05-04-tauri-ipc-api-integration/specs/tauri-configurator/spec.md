# tauri-configurator Specification

## ADDED Requirements

### Requirement: The UI MUST push intents through the Tauri IPC boundary
To maintain strict Zero-Panic validation and cryptographic integrity, the Vanilla JS frontend SHALL NOT manipulate the file system or local storage directly. It MUST send all JSON configuration payloads through the Tauri `invoke` IPC mechanism.

#### Scenario: Validating a UI payload in Rust
- **WHEN** the Vanilla JS routing controller dispatches an `apply_configuration` command
- **THEN** the Rust backend receives the intent asynchronously
- **AND** the backend verifies the schema integrity before taking any action on the data-plane.
