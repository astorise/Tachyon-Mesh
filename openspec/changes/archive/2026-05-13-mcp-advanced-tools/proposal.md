# Proposal: MCP Function Lifecycle & KV Integration

## Context
The P1 usability audit revealed a critical gap in the `tachyon-mcp` server's capabilities. While it exposes 14 tools, most are read-only (`tachyon_get_metrics`, `tachyon_hardware_status`) or limited to static manifests. It currently lacks the tools necessary to manage the actual lifecycle of WebAssembly functions or interact with the cluster's state (KV store).

## Problem
An LLM agent connected via MCP can observe the cluster but cannot take meaningful operational actions. It cannot deploy a pre-compiled `.wasm` artifact, check the runtime logs of a specific function, delete an obsolete function, or debug the state stored in `KV-Partition V2`. This limits the agent to an "observer" role rather than an "operator" role.

## Proposed Solution
Expand the `tachyon-mcp` tool registry to cover 80% of the cluster's core capabilities by implementing the following tools, mapping them directly to existing `tachyon_client` methods:
1. **WASM Lifecycle:** `tachyon_deploy_function`, `tachyon_list_functions`, `tachyon_delete_function`, `tachyon_function_logs`.
2. **State Management:** `tachyon_kv_get`, `tachyon_kv_put`, `tachyon_kv_delete`.
3. **Traffic Management:** `tachyon_canary_split` (to adjust routing weights between function versions).

## Impact
- **Agentic Autonomy:** Agents can fully deploy, test, debug, and teardown applications on the Tachyon mesh without human intervention in the CLI or UI.
- **Parity:** Brings the MCP server up to feature parity with `Tachyon-UI` and the CLI.