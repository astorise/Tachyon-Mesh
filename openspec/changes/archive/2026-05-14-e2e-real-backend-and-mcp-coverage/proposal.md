# Proposal: True E2E Coverage for UI and MCP

## Context
The post-Codex audit revealed a false sense of security in our current testing pipeline. While we have E2E files, Playwright is currently bypassing network calls by synthesizing DOM events (e.g., dispatching `iam:authenticated` directly). Similarly, the MCP test runner (`mcp_e2e_runner.rs`) only tests the initialization and read-only tools, ignoring the most critical mutating operations.

## Problem
1. **False Positives (UI):** If the `tachyon_client` (Rust) breaks its contract with the `core-host` API, the UI tests will still pass because they are mocking at the DOM level, not the network level.
2. **Untested Mutators (MCP):** Deploying a function, modifying KV state, or triggering rate limits via the MCP is currently completely untested in CI. A regression here would silently break AI agent integrations.

## Proposed Solution
1. **Network-Level Mocking (Playwright):** Implement MSW (Mock Service Worker) or Playwright's native `page.route()` to intercept real HTTP calls made by the Tauri backend and return fixed JSON fixtures. This forces the UI to process real network lifecycles.
2. **Behavioral MCP Tests:** Expand `mcp_e2e_runner.rs` to intentionally trigger rate-limit errors (`-32002`) and test the happy path for `deploy_function`, `kv_put`, and `canary_split`.

## Impact
- **Absolute Confidence:** Ensures the UI correctly handles network latency, parsing, and state transitions without bypassing the actual business logic.
- **Agent Reliability:** Guarantees that the MCP tool schemas and behaviors exactly match what an LLM expects.