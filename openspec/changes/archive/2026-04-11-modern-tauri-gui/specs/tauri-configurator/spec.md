## ADDED Requirements

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
