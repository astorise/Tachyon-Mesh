# Implementation Tasks

- [x] **Task 1: Documentation**
  - Create `docs/mcp-setup.md` with Claude Desktop and Cursor instructions.
  - Update root `README.md` to include a "Quickstart: LLM Agents (MCP)" section linking to the new doc.

- [x] **Task 2: Fix `tachyon_tail_logs` Schema**
  - Edit `tachyon-mcp/src/main.rs`.
  - Remove the `follow` parameter from the JSON schema definition of the tool.
  - Remove the parsing logic for `follow`.

- [x] **Task 3: Async Hardware Status**
  - Identify `tachyon_client::read_local_hardware_status()`.
  - Replace it with an async equivalent to prevent blocking the Tokio runtime.

- [x] **Task 4: Connection State Management**
  - Refactor the request handler in `tachyon-mcp/src/main.rs`.
  - Implement a `OnceLock` or pass a shared `State` containing the initialized `tachyon_client` to avoid reconnecting on every single RPC message.