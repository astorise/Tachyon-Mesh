## MODIFIED Requirements

### Requirement: Tauri v2 routes desktop builds through Vite commands
The `tachyon-ui/tauri.conf.json` configuration SHALL use the Tauri v2 build keys required to run the Vite development server and production build pipeline, and SHALL resolve packaged frontend assets from the crate-local `dist` directory.

#### Scenario: Tauri launches the Vite toolchain from the desktop crate
- **WHEN** Tauri reads `tachyon-ui/tauri.conf.json`
- **THEN** `build.beforeDevCommand` is `npm run dev`
- **AND** `build.beforeBuildCommand` is `npm run build`
- **AND** `build.devUrl` is `http://localhost:5173`
- **AND** `build.frontendDist` points to `dist`
- **AND** the resolved frontend asset directory stays inside the `tachyon-ui` crate
