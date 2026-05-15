# Proposal: Complete MCP E2E Test Coverage

## Context
The T+5 audit explicitly cleared Tachyon-Mesh for a Release Candidate (RC) but mandated complete End-to-End (E2E) testing for the MCP server before General Availability (GA). Currently, the `mcp_e2e_runner.rs` successfully tests rate limiting, initialization, and read-only tools, but completely misses the 7 advanced mutator/reader tools.

## Problem
1. **Untested Lifecycle:** Tools like `deploy_function`, `delete_function`, `list_functions`, and `function_logs` are core to the agentic DevOps use case but have no automated behavioral tests.
2. **Untested KV State:** While `kv_put` might be loosely tested, `kv_get` and `kv_delete` are ignored.
3. **Missing Error Assertions:** Agents rely on exact error codes to self-correct. We lack tests proving that malformed requests return exactly `-32602` (Invalid Params) and that disconnected hosts return `-32001` (Cluster Unreachable).

## Proposed Solution
Expand `tachyon-mcp/tests/mcp_e2e_runner.rs` to include a deterministic sequence of operations that mimics an autonomous agent deploying and tearing down a workload:
1. Generate a dummy `.wasm` artifact locally.
2. Test `deploy_function` with this artifact.
3. Test `list_functions` to verify deployment.
4. Test `kv_put`, `kv_get`, and `kv_delete`.
5. Test `delete_function`.
6. Inject malformed JSON to assert `-32602`.

## Impact
- **Agentic Reliability:** Absolute proof that the JSON-RPC contract is respected across the entire operational surface.
- **GA Readiness:** Clears the second of three major blockers for Enterprise production release.