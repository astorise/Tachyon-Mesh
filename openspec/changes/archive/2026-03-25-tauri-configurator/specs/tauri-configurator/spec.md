## ADDED Requirements

### Requirement: Tauri configurator supports headless manifest generation from the CLI
The `tachyon-cli` application SHALL expose a `generate` subcommand that accepts route and memory inputs, invokes the Rust manifest-generation backend, and exits without opening a desktop window when run from the terminal.

#### Scenario: CLI mode exits before launching a webview
- **WHEN** a developer invokes `tachyon-cli generate --route /api/guest-example --memory 64`
- **THEN** the application parses the CLI arguments before any desktop window is created
- **AND** the manifest-generation backend runs to completion using those arguments
- **AND** the process exits with a success or failure status code without opening a webview
