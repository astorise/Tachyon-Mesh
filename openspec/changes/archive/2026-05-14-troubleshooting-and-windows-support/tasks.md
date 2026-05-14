# Implementation Tasks

- [x] **Task 1: Create `TROUBLESHOOTING.md`**
  - Draft the document covering the 15 specific failure modes mentioned in the audit.
  - Update `README.md` to link to it prominently.

- [x] **Task 2: Windows Release Pipeline**
  - Edit `.github/workflows/release.yml`.
  - Add `x86_64-pc-windows-msvc` to the build matrix.
  - Update the packaging step to generate a `.zip` file for Windows artifacts.

- [x] **Task 3: MCP Schema Warning**
  - Edit `tachyon-mcp/src/main.rs`.
  - Add `tracing::warn!` on schema fetch failure.
  - Modify the `tools/list` RPC endpoint to append the `data.warnings` array if the strict schema is not loaded.
