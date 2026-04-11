# Tasks: Change 064 Implementation

**Agent Instruction:** Treat `tachyon-ui` as a pure GUI crate. Do not leave any startup path that inspects CLI arguments before Tauri initializes.

## [TASK-1] Clean UI Dependencies
- [x] Remove CLI-only and manifest-generation dependencies from `tachyon-ui/Cargo.toml`.
- [x] Keep only the dependencies required for the Tauri desktop wrapper and `tachyon-client`.

## [TASK-2] Exorcise the UI Entrypoint
- [x] Replace the current `tachyon-ui` entrypoint with a direct `tauri::Builder` bootstrap in `tachyon-ui/src/main.rs`.
- [x] Keep the Windows subsystem attribute at the absolute top of `tachyon-ui/src/main.rs`.
- [x] Remove the startup-time `std::env::args` / manifest-generation path from `tachyon-ui`.
- [x] Remove obsolete CLI plugin configuration from the Tauri desktop project.

## [TASK-3] Secure MCP stdio
- [x] Ensure `tachyon-mcp` emits JSON-RPC responses to `stdout` only.
- [x] Ensure diagnostics and unexpected failures are written to `stderr`.

## [TASK-4] Build Pipeline Cleanup
- [x] Stop invoking `tachyon-ui` as a manifest generator from build infrastructure.
- [x] Keep the renamed desktop project buildable after the cleanup.

## Validation Step
- [x] Run `cargo check -p tachyon-ui`.
- [x] Run `cargo check -p tachyon-mcp`.
- [x] Run `cargo build`.
- [x] Run `npm run tauri build` in `tachyon-ui`.
- [x] Run `openspec validate --all`.
