## MODIFIED Requirements

### Requirement: Tachyon desktop UI is built through a Vite frontend pipeline
The `tachyon-ui` desktop application SHALL build its frontend through a Vite-based toolchain rooted in the `tachyon-ui` directory and SHALL preserve the injected Tailwind CSS and GSAP frontend assets inside that flattened crate layout.

#### Scenario: Desktop frontend assets stay in the `tachyon-ui` crate
- **WHEN** the Tauri desktop application is built
- **THEN** the frontend entry point is `tachyon-ui/index.html`
- **AND** the frontend logic entry point is `tachyon-ui/src/main.ts`
- **AND** the frontend styling entry point is `tachyon-ui/src/style.css`
- **AND** `tachyon-ui/package.json` includes Vite, Tailwind CSS, and GSAP for that frontend bundle

### Requirement: The desktop frontend can invoke a Rust status command
The `tachyon-ui` Rust backend SHALL expose a Tauri command named `get_engine_status`, bootstrap directly through `tauri::Builder`, and delegate the status query to `tachyon-client`.

#### Scenario: The frontend requests the engine status through the clean-slate wrapper
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the desktop runtime dispatches the command through `tauri::generate_handler!`
- **AND** the Rust implementation awaits `tachyon_client::get_engine_status()`
- **AND** no CLI-only startup path is evaluated before the desktop window is initialized

### Requirement: The desktop wrapper launches without evaluating CLI startup arguments
The `tachyon-ui` project SHALL bootstrap the Tauri runtime immediately on startup and SHALL NOT inspect `std::env::args`, `clap`, or any equivalent CLI parser before the desktop webview is created.

#### Scenario: The GUI binary starts directly in desktop mode
- **WHEN** a user launches `tachyon-ui`
- **THEN** the process enters `tauri::Builder` immediately
- **AND** no manifest-generation or route-parsing code runs before the desktop window is initialized

### Requirement: The desktop wrapper excludes legacy CLI plugin wiring
The `tachyon-ui` project SHALL NOT retain Tauri CLI plugin wiring or desktop config intended for manifest-generation subcommands.

#### Scenario: Tauri config contains no desktop CLI plugin section
- **WHEN** the desktop project configuration is loaded from `tachyon-ui/tauri.conf.json`
- **THEN** the configuration does not declare a `plugins.cli` manifest-generation section
- **AND** the desktop Rust entrypoint does not register `tauri_plugin_cli`

## REMOVED Requirements

### Requirement: Tauri configurator supports headless manifest generation from the CLI
**Reason**: The repository no longer ships a `tachyon-cli` workspace crate, and the desktop project has been reduced to a pure GUI wrapper.
**Migration**: Treat manifest creation as dedicated external tooling and feed `core-host` a valid signed `integrity.lock`.

### Requirement: CLI mode seals route SemVer metadata
**Reason**: The removed `tachyon-cli` manifest-generation path no longer exists in the workspace.
**Migration**: Encode SemVer metadata in the signed `integrity.lock` supplied to the host by dedicated manifest tooling.

### Requirement: Tauri configurator supports headless manifest generation from the desktop wrapper
**Reason**: `tachyon-ui` no longer owns manifest sealing and starts directly in desktop mode.
**Migration**: Use dedicated manifest tooling before launching the desktop wrapper or building `core-host`.

## ADDED Requirements

### Requirement: The desktop wrapper keeps a clean-slate Rust dependency surface
The `tachyon-ui` Rust crate SHALL depend only on the shared `tachyon-client` library plus the Tauri runtime and build crates needed for desktop bootstrap, and SHALL NOT pull in legacy CLI or manifest-generation dependencies.

#### Scenario: The Rust crate does not reintroduce CLI-only dependencies
- **WHEN** a developer inspects `tachyon-ui/Cargo.toml`
- **THEN** the runtime dependencies include `tachyon-client` and `tauri`
- **AND** the build dependencies include `tauri-build`
- **AND** the crate does not depend on `clap` or manifest-signing crates
