# Specification: MCP Server Resiliency and Authentication

## 1. Strict PAT Validation
The Personal Access Token (PAT) is currently only validated at process startup.
* **Per-Request Validation:** Move the PAT validation logic into the JSON-RPC request middleware/handler. The token must be verified against the host's active session state for *every* incoming request.
* **Environment Enforcement:** `TACHYON_MCP_URL` and `TACHYON_MCP_PAT` must be strictly enforced for external calls. For local tools, the token must still match the host's local administrative context.
* **Cleanup:** Remove the dead-code `_token` variable.

## 2. Per-Tool Rate Limiting
The current rate limiter uses a single, global Token Bucket locked by a Mutex, punishing lightweight reads identically to heavy writes.
* **Granular Limits:** Implement a `HashMap<String, TokenBucket>` where the key is the MCP tool name.
    * `tachyon_apply_manifest`, `tachyon_seal_overlay`: 1 request / minute.
    * `tachyon_get_metrics`, `tachyon_tail_logs`: 30 requests / minute.
    * `tachyon_register_resource`: 10 requests / minute.
* **Panic Removal:** Remove `.expect("write limiter should not be poisoned")`. If the Mutex is poisoned, return a structured JSON-RPC error `-32603` (Internal Error) rather than crashing the server.
* **Short-Lived Persistence:** Write the token bucket state to a `.lock` or `.state` file in the temporary directory (updated every ~10s or upon bucket exhaustion) to prevent an attacker from bypassing the limit by restarting the MCP process.