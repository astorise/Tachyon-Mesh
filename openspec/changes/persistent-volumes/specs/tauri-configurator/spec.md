## MODIFIED Requirements

### Requirement: Tauri configurator supports headless manifest generation from the CLI
The `tachyon-cli` application SHALL expose a `generate` subcommand that accepts
regular route, privileged system route, secret grant, memory, optional
per-route scaling inputs, and optional volume mappings using
`[/route=]HOST:GUEST[:ro|rw]`, invokes the Rust manifest-generation backend, and
exits without opening a desktop window when run from the terminal.

#### Scenario: CLI mode seals a route-specific volume
- **WHEN** a developer invokes `tachyon-cli generate --route /api/guest-volume --volume /api/guest-volume=/tmp/tachyon_data:/app/data:ro --memory 64`
- **THEN** the generated canonical configuration payload includes `/api/guest-volume`
- **AND** the route includes a volume entry with `guest_path = /app/data`
- **AND** the route volume is marked `readonly = true`
- **AND** the command succeeds without opening a webview

#### Scenario: CLI mode rejects implicit volumes when multiple routes exist
- **WHEN** a developer invokes `tachyon-cli generate --route /api/guest-example --route /api/guest-volume --volume /tmp/tachyon_data:/app/data:rw --memory 64`
- **THEN** the command exits with a failure status
- **AND** the error explains that the volume must target a declared sealed route explicitly
