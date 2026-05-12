# Proposal: Tachyon-MCP Unblocking & Documentation

## Context
The recent usability audit flagged the `tachyon-mcp` component as functionally inert for new users due to a complete lack of documentation (P0). Additionally, critical runtime bugs were found in `tachyon-mcp/src/main.rs`, including an executor-blocking synchronous call, inefficient connection recreation on every request, and a broken contract regarding log following.

## Problem
1. **Discoverability:** Agents and developers cannot use the MCP server because there is no `claude_desktop_config.json` example, nor is it mentioned in the `README.md`.
2. **Performance/Blocking:** `tachyon_client::read_local_hardware_status()` is called synchronously inside an async Tokio context (line 346), which stalls the runtime under load. `set_connection()` is re-invoked on every RPC call (line 541).
3. **Contract Violation:** The `tachyon_tail_logs` tool exposes a `follow: boolean` parameter but ignores it (line 668), degrading agent reliability.

## Proposed Solution
1. **Documentation:** Create `docs/mcp-setup.md` and link it in the main `README.md` with copy-pasteable configuration snippets.
2. **Async & State Fixes:** - Refactor `read_local_hardware_status()` to be properly `.await`ed.
   - Introduce a connection cache/singleton in `tachyon-mcp` to reuse the `core-host` connection instead of rebuilding it per request.
3. **Schema Alignment:** Remove the `follow` parameter from the `tachyon_tail_logs` tool schema until streaming is natively supported via JSON-RPC/SSE notifications.

## Impact
- **Agentic Usability:** Takes setup time from "impossible" to < 2 minutes.
- **Stability:** Prevents Tokio thread starvation and drastically reduces latency per tool call.