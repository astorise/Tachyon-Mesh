# Proposal: MCP Error Taxonomy & Agent Recovery

## Context
The usability audit (P1) highlighted that the `tachyon-mcp` server currently wraps almost all failures in an opaque `-32603` (Internal Error) JSON-RPC code. It lacks granularity for common failures like invalid parameters, network unreachability, or rate limiting.

## Problem
When an LLM agent receives an opaque `-32603` error, it cannot programmatically determine the root cause. 
- If a manifest parameter is misspelled, it doesn't get a `-32602` (Invalid Params) with the field name.
- If the cluster is down, it doesn't get a specific "Unreachable" code, leading to infinite retries.
- If it hits a rate limit, it doesn't receive a `reset_at` or `retry_after` payload, causing immediate failed retries.
This drastically lowers the reliability and autonomy of the agent.

## Proposed Solution
1. **JSON-RPC Error Mapping:** Implement standard JSON-RPC 2.0 error codes in `tachyon-mcp/src/main.rs`.
   - `-32602`: Invalid Params (e.g., malformed manifest, missing required fields).
   - `-32001`: Cluster Unreachable (Core-Host is down or network timeout).
   - `-32002`: Rate Limited / Too Many Requests.
2. **Structured `data` Payload:** Enhance the JSON-RPC error response to include a `data` object containing actionable metadata (e.g., the specific field that failed validation, or the `retry_after` timestamp for rate limits).

## Impact
- **Agentic Resilience:** LLMs can parse the error code and the `data` payload to automatically correct their next tool call, pause execution until the rate limit lifts, or gracefully inform the user that the cluster is down.