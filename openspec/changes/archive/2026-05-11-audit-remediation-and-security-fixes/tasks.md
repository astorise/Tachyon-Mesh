# Tasks

## 1. Security & Storage (Tauri)
- [x] Refactor `write_secure_profile` and `read_secure_profile` in `tachyon-ui/src/main.rs` to utilize the Tauri Stronghold API (`insert_record`, `get_record`).
- [x] Remove all plaintext writes of passwords and PATs to disk.
- [x] Uncheck "Remember credentials" by default and add a UI warning if enabled without Stronghold backend availability.
- [x] Implement short-lived MFA session tokens (issued by host) to allow Step-Up MFA without locally stored passwords.

## 2. Core Host (Backend)
- [x] Implement `GET /admin/metrics` in `host_core/app_runtime.rs`.
- [x] Implement `GET /admin/shadow/diffs` in `host_core/app_runtime.rs`.
- [x] Implement `POST /admin/chaos/scenarios` in `host_core/app_runtime.rs`.

## 3. MCP Integration
- [x] Update PAT validation in `tachyon-mcp/src/main.rs` to verify tokens per-request instead of only at startup.
- [x] Make `TACHYON_MCP_URL` mandatory or enforce strict local session checks for the token.
- [x] Replace the shared Token Bucket with a per-tool rate limit strategy (e.g., stricter limits for `apply_manifest`).
- [x] Remove the `.expect()` on the rate limiter Mutex; return a clean RPC error on lock poisoning.
- [x] Add basic JSON-RPC round-trip integration tests for `tachyon-mcp`, including rate-limit denial scenarios.

## 4. UI, Canvas & Validation
- [x] Update `tachyon-ui/src/main.rs` to actually use `wit_bindgen!` generated code for `validate_traffic_config` instead of raw JSON checks.
- [x] Include the 4 remaining `config-*.wit` bindings and wire them to their respective validation logic.
- [x] Create `applyAndSeal(domain, payload)` in `tachyon-ui/src/utils/network.ts` to orchestrate diffing, confirmation, and sequential sealing/applying.
- [x] Update `requiresStepUp` to utilize the new atomic flow.
- [x] Update `TachyonObservabilityPanel.ts` to render live data from the newly implemented metrics, logs, and shadow diffs endpoints.
- [x] Add unit/integration tests for `TachyonBundleConflictModal.ts`.
