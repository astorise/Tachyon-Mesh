# Proposal: MCP Mutator Rate-Limiting & Schema Hardening

## Context
The post-Codex usability audit identified two critical P0 vulnerabilities in the newly expanded `tachyon-mcp` server. While read-only tools are safe, the 6 new mutating tools (`deploy_function`, `delete_function`, `kv_put`, `kv_delete`, `canary_split`, `function_logs`) lack execution constraints and clear semantic boundaries.

## Problem
1. **Denial of Service Risk:** An autonomous LLM agent stuck in a reasoning loop could spam `deploy_function` or `canary_split`, overwhelming the cluster, exhausting VRAM, or breaking production traffic.
2. **Agentic Hallucination:** The JSON schemas for these tools are missing `required` arrays (e.g., `kv_put` missing key/namespace) and lack LLM-friendly descriptions (e.g., the agent doesn't know if the WASM artifact should be a local path, base64, or URL; it doesn't know `weight_pct=0` means rollback).

## Proposed Solution
1. **Granular Rate Limiting:** Implement strict, tool-specific rate limits in `main.rs` (e.g., `canary_split`: 2/min, `deploy_function`: 5/min, `kv_put/delete`: 30/min).
2. **Schema Hardening:** Update the `list_tools` endpoint to include all `required` arrays and highly prescriptive `description` fields designed specifically to guide LLM reasoning.

## Impact
- **Security:** Prevents accidental or malicious infrastructure destruction by rogue agents.
- **Reliability:** The agent succeeds on the first try because the tool contract explicitly defines the expected data formats.