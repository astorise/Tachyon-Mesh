# tauri-configurator Specification

## ADDED Requirements

### Requirement: The UI Backend MUST strictly validate intents against WIT definitions
The Rust Tauri backend SHALL NOT act as a simple passthrough proxy for JSON payloads. Before dispatching any configuration to the `system-faas-gossip` network, the backend MUST deserialize and validate the JSON payload against the strict Rust structures generated from the `.wit` contracts (e.g., `config-routing.wit`).

#### Scenario: Submitting a malformed route configuration
- **GIVEN** the UI submits a JSON payload for `config-routing` missing the required `bind_address` in a Gateway object
- **WHEN** the Rust backend receives the payload via IPC
- **THEN** the strict Serde deserialization fails
- **AND** the backend returns a safe, handled failure response to the UI
- **AND** the data-plane remains untouched.
