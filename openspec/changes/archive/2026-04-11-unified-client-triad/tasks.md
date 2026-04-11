# Tasks: Change 062 Implementation

**Agent Instruction:** Keep the workspace buildable throughout the refactor. Rename paths and package identifiers consistently; do not leave split-brain references to both `tachyon-cli` and `tachyon-ui`.

## [TASK-1] Shared Client Crate
- [x] Create a new workspace library crate named `tachyon-client`.
- [x] Add async shared APIs for lockfile access and engine status reporting.
- [x] Move the workspace-root / `integrity.lock` reading logic out of the desktop wrapper into `tachyon-client`.

## [TASK-2] Desktop Wrapper Rename
- [x] Rename the `tachyon-cli` project directory to `tachyon-ui`.
- [x] Rename the Cargo package to `tachyon-ui` and add a dependency on `tachyon-client`.
- [x] Add the Windows GUI subsystem attribute at the top of `tachyon-ui/src/main.rs`.
- [x] Refactor the Tauri `get_engine_status` command to call `tachyon_client::get_engine_status()`.

## [TASK-3] MCP Wrapper
- [x] Create a new workspace binary crate named `tachyon-mcp`.
- [x] Implement a basic JSON-RPC loop over `stdin` / `stdout`.
- [x] Expose a `tools/call` handler for `tachyon_mesh_status` backed by `tachyon_client::get_engine_status()`.
- [x] Expose a `tools/call` handler for lockfile inspection backed by `tachyon_client::read_lockfile()`.

## [TASK-4] Workspace and Pipeline Updates
- [x] Update the root workspace members to include `tachyon-client`, `tachyon-ui`, and `tachyon-mcp`.
- [x] Update CI, release workflow, Docker build steps, and project paths to use `tachyon-ui`.
- [x] Update package metadata, Tauri metadata, and OpenSpec references so the renamed desktop project remains coherent.

## Validation Step
- [x] Run `cargo check --workspace`.
- [x] Run `cargo build`.
- [x] Run `openspec validate --all`.
