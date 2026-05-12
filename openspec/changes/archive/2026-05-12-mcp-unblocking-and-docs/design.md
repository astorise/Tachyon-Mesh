# Design: Tachyon-MCP Unblocking & Documentation

## Approach

Four independent changes with no coupling between them. The Rust fixes target `tachyon-mcp/src/main.rs` only; the documentation additions are purely additive.

### 1. Documentation

`docs/mcp-setup.md` is a self-contained setup guide covering Claude Desktop, Cursor, and raw stdio transport. It includes the copy-pasteable JSON config snippet, a table of all available tools, and a troubleshooting section. No build step is needed; the file is read directly by IDE or browser.

`README.md` gains a new `## Quickstart: LLM Agents (MCP)` section above `## Performance & Benchmarks` with the 1-minute config example and a link to the full guide.

### 2. `tachyon_tail_logs` schema alignment

The `follow: boolean` property was defined in the JSON schema but never honoured — the handler always returned a fixed snapshot and included `followRequested` in `structuredContent`, which agents could interpret as a promise. Removing the field from both the schema and the handler eliminates the broken contract. The description is updated to explicitly state that continuous streaming is not supported over stdio MCP.

### 3. Async hardware status (`spawn_blocking`)

`tachyon_client::read_local_hardware_status()` calls `sysinfo::System::refresh_memory()`, which performs kernel syscalls and may block the calling OS thread for milliseconds. Inside a Tokio async context, this starves other tasks sharing the same thread. Both call sites (the `resources/read` MCP resource handler and the `tachyon_hardware_status` tool handler) are updated to use `tokio::task::spawn_blocking(...)`, which offloads the call to Tokio's dedicated blocking thread pool. The resulting `JoinHandle` is `.await`ed so the async API surface is unchanged.

No change to `tachyon_client` is required; the fix is entirely in the MCP server.

### 4. Connection caching (`OnceLock`)

`validate_request_auth` called `set_connection` on every JSON-RPC request. `set_connection` performs an HTTP round-trip to `fetch_remote_status` on the `core-host` just to validate the PAT. For a 100-tool-call agent session this is 100 unnecessary HTTP requests.

A module-level `static CONNECTION_INITIALIZED: OnceLock<()>` guards the call: the first request that passes through `validate_request_auth` triggers `set_connection` and marks the cell as set. All subsequent requests return early. The `tachyon_client` global connection state (`connection_state()` `RwLock`) persists across calls within the same process, so tool calls after the first still have a valid connection.

The serial nature of the stdio MCP transport means there is no race condition: only one request is in-flight at any given time.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| `follow` removal | Remove entirely | Keep with `// TODO` comment | A documented-but-broken contract is worse than no contract; agents will retry indefinitely if they believe streaming is available |
| Async hw status | `spawn_blocking` in MCP server | Add async wrapper in `tachyon_client` | Avoids changing the stable public API of `tachyon_client`; the blocking behaviour is only a problem in the async context |
| Connection cache | `OnceLock<()>` | `tokio::sync::OnceCell` | Stdlib OnceLock is sufficient; the serial stdio transport means no async concurrent initialization race |
| Connection failure recovery | Not handled | Re-try on each request | MCP sessions are short-lived; if the initial connection fails the agent will see an error and can restart the server |
