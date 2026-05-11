# Proposal: Audit Remediation and Security Fixes

## Problems

A recent audit highlighted significant functional and security gaps that undermine the integrity of the Tachyon mesh system. The critical issues include:

1. **Security Theater in Stronghold (REG-1):** The `write_secure_profile` implementation in Tauri simply writes plaintext JSON to disk. The `tauri-plugin-stronghold` is loaded but never used to encrypt/store records. Furthermore, Step-Up MFA relies on this plaintext password leakage.
2. **Phantom Endpoints (REG-2):** `tachyon-client` and MCP tools promise access to metrics, shadow diffs, and chaos scenarios, but the actual routes (`GET /admin/metrics`, `GET /admin/shadow/diffs`, `POST /admin/chaos/scenarios`) are entirely absent from `core-host/src/host_core/app_runtime.rs`. This results in silent 404 errors.
3. **Dead WIT Validation (REG-3):** Configuration validation in the UI relies on manual JSON string-keyed checks rather than the generated `wit_bindgen` contracts, rendering the WIT definitions meaningless as a source of truth.
4. **Flawed MCP Authentication (REG-4):** Personal Access Tokens (PAT) are validated only once on startup. Expiration is ignored. If `TACHYON_MCP_URL` is omitted, the PAT is never effectively validated, creating an authentication bypass vulnerability for local operations.
5. **Brittle and Unfair Rate Limiting (REG-5):** The MCP uses a shared, non-persistent Token Bucket for all write operations, punishing light local writes identically to heavy `apply_manifest` operations. It also uses a panic-inducing `.expect()` on the Mutex.
6. **UX Frictions:** Apply flows require manual 2-step processes (apply then seal), MFA step-ups fail unless "Remember credentials" is checked, observability panels lack live data integration, and `tachyon-mcp` lacks basic integration testing.

## What Changes

### 1. Tauri & Security (Stronghold & MFA)
- **Real Stronghold Integration:** Rewrite `read_secure_profile` and `write_secure_profile` in `tachyon-ui/src/main.rs` to actually invoke `app.handle().state::<Stronghold>().insert_record()` and `.get_record()`.
- **Stateless Step-Up MFA:** Disconnect Step-Up MFA from the plaintext password. The core host must issue a short-lived MFA Session Token (or cookie) upon valid TOTP verification, allowing step-ups without requiring the user to "Remember credentials".

### 2. Core Host (Phantom Endpoints)
- Implement the missing endpoints in `host_core/app_runtime.rs`:
  - `GET /admin/metrics` returning `telemetry::TelemetrySnapshot`.
  - `GET /admin/shadow/diffs` returning events from the shadow proxy log.
  - `POST /admin/chaos/scenarios` hooking into the chaos harness.

### 3. MCP Resiliency & Security
- **PAT Validation:** Enforce `TACHYON_MCP_URL` or validate the PAT strictly against the local host session. Verify the token on every JSON-RPC request rather than just at startup. Remove the `_token` dead-code smell.
- **Per-Tool Rate Limiting:** Implement a granular rate limiter (e.g., 5/min for local writes, 1/min for `apply_manifest`). Remove the `.expect()` from the Mutex and handle lock poisoning gracefully. Use temporary file locks for short-lived persistence to prevent bypass via restarts.

### 4. UI & Frontend Refinements
- **WIT-Based Validation:** Refactor `validate_traffic_config` to deserialize configurations through the generated `traffic-management-config` world. Ensure the other 4 `config-*.wit` files are also generated and utilized.
- **Atomic Apply Flow:** Introduce `applyAndSeal(domain, payload)` in `network.ts`. This orchestrator will bundle the config, display a visual diff, prompt for confirmation (and MFA if needed), and then execute the seal & apply sequentially.
- **Observability Panel:** Update `TachyonObservabilityPanel` to actually consume and render data from `get_metrics`, `tail_logs`, and `get_shadow_diffs`.

### 5. Testing
- Add JSON-RPC round-trip integration tests for `tachyon-mcp`.
- Add integration tests for the `TachyonBundleConflictModal`.