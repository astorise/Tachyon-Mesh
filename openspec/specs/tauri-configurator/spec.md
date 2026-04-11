# tauri-configurator Specification

## Purpose
TBD - created by archiving change tauri-configurator. Update Purpose after archive.
## Requirements
### Requirement: Tauri configurator supports headless manifest generation from the CLI
The `tachyon-cli` application SHALL expose a `generate` subcommand that accepts regular route,
privileged system route, secret grant, memory, optional per-route scaling inputs, and optional
volume mappings using `[/route=]HOST:GUEST[:ro|rw]`, invokes the Rust manifest-generation backend,
and exits without opening a desktop window when run from the terminal.

#### Scenario: CLI mode seals route scaling overrides
- **WHEN** a developer invokes `tachyon-cli generate --route /api/guest-example --route-scale /api/guest-example=1:8 --memory 64`
- **THEN** the generated canonical configuration payload includes `/api/guest-example` with `min_instances = 1`
- **AND** the generated canonical configuration payload includes `/api/guest-example` with `max_concurrency = 8`
- **AND** the command succeeds without opening a webview

#### Scenario: CLI mode rejects scaling overrides for unknown routes
- **WHEN** a developer invokes `tachyon-cli generate --route /api/guest-example --route-scale /api/missing=1:8 --memory 64`
- **THEN** the command exits with a failure status
- **AND** the error explains that the scaling override must target a declared sealed route

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

### Requirement: CLI mode seals route SemVer metadata
The `tachyon-cli` application SHALL allow `generate` callers to seal logical route names,
semantic versions, and dependency constraints without opening the desktop UI.

#### Scenario: CLI mode records route identity and dependencies
- **WHEN** a developer invokes `tachyon-cli generate --route /api/faas-a --route-name /api/faas-a=faas-a --route-version /api/faas-a=2.0.0 --route-dependency /api/faas-a=faas-b@^3.1.0 --memory 64`
- **THEN** the generated canonical configuration payload includes `/api/faas-a`
- **AND** the route entry records `name = "faas-a"` and `version = "2.0.0"`
- **AND** the route entry records a dependency map containing `faas-b = "^3.1.0"`
- **AND** the command succeeds without opening a webview

#### Scenario: CLI mode rejects an invalid dependency requirement
- **WHEN** a developer invokes `tachyon-cli generate --route /api/faas-a --route-dependency /api/faas-a=faas-b@not-semver --memory 64`
- **THEN** the command exits with a failure status
- **AND** the error explains that the dependency requirement is not valid SemVer syntax

### Requirement: Tachyon desktop UI is built through a Vite frontend pipeline
The `tachyon-cli` desktop application SHALL build its frontend through a Vite-based toolchain rooted in the `tachyon-cli` directory and SHALL use Tailwind CSS and GSAP in that frontend bundle.

#### Scenario: Desktop frontend assets are sourced from Vite entry points
- **WHEN** the Tauri desktop application is built
- **THEN** the frontend entry point is `tachyon-cli/index.html`
- **AND** the frontend logic entry point is `tachyon-cli/src/main.ts`
- **AND** the frontend styling entry point is `tachyon-cli/src/style.css`
- **AND** the Vite dev server runs on port `5173`

### Requirement: Tauri v2 routes desktop builds through Vite commands
The `tachyon-cli/tauri.conf.json` configuration SHALL use the Tauri v2 build keys required to run the Vite development server and production build pipeline.

#### Scenario: Tauri launches the Vite toolchain
- **WHEN** Tauri reads `tachyon-cli/tauri.conf.json`
- **THEN** `build.beforeDevCommand` is `npm run dev`
- **AND** `build.beforeBuildCommand` is `npm run build`
- **AND** `build.devUrl` is `http://localhost:5173`
- **AND** `build.frontendDist` points to `dist`

### Requirement: The desktop frontend can invoke a Rust status command
The `tachyon-cli` Rust backend SHALL expose a Tauri command named `get_engine_status` that the frontend can invoke without opening a CLI-only execution path.

#### Scenario: The frontend requests the engine status
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the Tauri runtime dispatches the command through the registered invoke handler
- **AND** the command returns a mocked status payload compatible with the dashboard UI

