# Title: Audit Closure: Type Safety, MCP Security, and Error Localization

## Problem Statement
This change addresses the final architectural and security findings from the recent codebase audit:
1. **Silent WIT Divergence:** UI configuration structs (e.g., `AiConfig`, `TrafficConfig`) in `tachyon-ui/src/main.rs` are manually derived from JSON, ignoring the actual `wit/config-*.wit` source of truth.
2. **Unprotected MCP Server:** `tachyon-mcp` assumes local trust, lacking authentication for remote AI agents and lacking rate-limiting for write operations, exposing the mesh to LLM hallucinations or abuse.
3. **UI Vulnerabilities:** Potential XSS vectors exist where `innerHTML` is used with unescaped data in Web Components, and `tachyon-ui/src/main.rs` ends with a hard panic (`expect`) instead of a graceful exit.
4. **Opaque Backend Errors:** While the UI has i18n support, errors bubbling up from the Rust backend are displayed raw to the user, creating a jarring UX for non-English speakers.

## Objective
1. Enforce strict compile-time contracts by generating Rust types directly from WIT files using `wit-bindgen`.
2. Secure the MCP server with PAT-based authentication and write-operation rate limiting.
3. Harden the UI against XSS and implement graceful Tauri app termination.
4. Create an Error Translation interceptor module to parse and localize Rust backend errors before displaying them via toasts.