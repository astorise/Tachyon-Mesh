# tauri-configurator Specification

## Purpose
TBD - created by archiving change tauri-configurator. Update Purpose after archive.
## Requirements
### Requirement: Tachyon desktop UI is built through a Vite frontend pipeline
The `tachyon-ui` desktop application SHALL build its frontend through a Vite-based toolchain rooted in the `tachyon-ui` directory and SHALL preserve the injected Tailwind CSS and GSAP frontend assets inside that flattened crate layout.

#### Scenario: Desktop frontend assets stay in the `tachyon-ui` crate
- **WHEN** the Tauri desktop application is built
- **THEN** the frontend entry point is `tachyon-ui/index.html`
- **AND** the frontend logic entry point is `tachyon-ui/src/main.ts`
- **AND** the frontend styling entry point is `tachyon-ui/src/style.css`
- **AND** `tachyon-ui/package.json` includes Vite, Tailwind CSS, and GSAP for that frontend bundle

### Requirement: Tauri v2 routes desktop builds through Vite commands
The `tachyon-ui/tauri.conf.json` configuration SHALL use the Tauri v2 build keys required to run the Vite development server and production build pipeline, and SHALL resolve packaged frontend assets from the crate-local `dist` directory.

#### Scenario: Tauri launches the Vite toolchain from the desktop crate
- **WHEN** Tauri reads `tachyon-ui/tauri.conf.json`
- **THEN** `build.beforeDevCommand` is `npm run dev`
- **AND** `build.beforeBuildCommand` is `npm run build`
- **AND** `build.devUrl` is `http://localhost:5173`
- **AND** `build.frontendDist` points to `dist`
- **AND** the resolved frontend asset directory stays inside the `tachyon-ui` crate

### Requirement: The desktop frontend can invoke a Rust status command
The `tachyon-ui` Rust backend SHALL expose a Tauri command named `get_engine_status`, bootstrap directly through `tauri::Builder`, and delegate the status query to `tachyon-client`.

#### Scenario: The frontend requests the engine status through the clean-slate wrapper
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the desktop runtime dispatches the command through `tauri::generate_handler!`
- **AND** the Rust implementation awaits `tachyon_client::get_engine_status()`
- **AND** no CLI-only startup path is evaluated before the desktop window is initialized

### Requirement: The desktop frontend can invoke shared client status queries
The `tachyon-ui` Rust backend SHALL delegate status queries to the shared `tachyon-client` library instead of embedding duplicated lockfile reading logic in the Tauri wrapper.

#### Scenario: The frontend requests the engine status
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the Tauri command awaits `tachyon_client::get_engine_status()`
- **AND** the returned payload comes from the shared client layer

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

### Requirement: The desktop wrapper keeps a clean-slate Rust dependency surface
The `tachyon-ui` Rust crate SHALL depend only on the shared `tachyon-client` library plus the Tauri runtime and build crates needed for desktop bootstrap, and SHALL NOT pull in legacy CLI or manifest-generation dependencies.

#### Scenario: The Rust crate does not reintroduce CLI-only dependencies
- **WHEN** a developer inspects `tachyon-ui/Cargo.toml`
- **THEN** the runtime dependencies include `tachyon-client` and `tauri`
- **AND** the build dependencies include `tauri-build`
- **AND** the crate does not depend on `clap` or manifest-signing crates

### Requirement: The desktop UI switches management planes without reloading
The `tachyon-ui` frontend SHALL bind sidebar navigation links to pre-rendered management-plane views and switch between them inside the existing `<main>` container without a full page reload.

#### Scenario: The operator selects a different management plane
- **WHEN** the operator clicks a sidebar link for Dashboard, Mesh Topology, FaaS Deployments, Identity, or AI Broker
- **THEN** the currently visible panel fades and slides out through GSAP
- **AND** the selected panel fades and slides in within the same page shell
- **AND** the selected sidebar link becomes the active link

### Requirement: The desktop UI exposes dedicated panels for topology, deployment, identity, and AI workflows
The `tachyon-ui` frontend SHALL expose dedicated panels for mesh topology, FaaS deployments, identity posture, and AI model brokerage using the shared Tauri commands and widgets already owned by the desktop client.

#### Scenario: The operator opens Mesh Topology
- **WHEN** the Mesh Topology panel becomes active
- **THEN** the frontend invokes `get_mesh_graph`
- **AND** it renders the returned route and batch-target snapshot in the topology view

#### Scenario: The operator opens FaaS Deployments
- **WHEN** the FaaS Deployments panel becomes active
- **THEN** the dashboard content is replaced by a panel labeled `Deployment Manager (Ready)`
- **AND** the asset upload controls remain available in that panel

#### Scenario: The operator opens Identity
- **WHEN** the Identity panel becomes active
- **THEN** the frontend renders the administrative user table and MFA recovery posture from the existing onboarding workflow

#### Scenario: The operator opens AI Broker
- **WHEN** the AI Broker panel becomes active
- **THEN** the frontend renders the chunked model upload controls and progress bar in that panel

