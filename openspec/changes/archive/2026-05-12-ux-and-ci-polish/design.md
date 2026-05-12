# Design: UX Polish and Cross-Layer CI Validation

## Approach

Two independent workstreams with no shared state or runtime coupling. Each can be merged independently.

### 1. Guided Tour & Connection Store

**Tour step ordering** — The new "Atomic Seal & Apply" step is inserted after the registry overview so the user has already seen what assets exist before being introduced to the sign-off flow. Step sequence: auth → header → live overview → supply-chain registry → seal & apply → observability.

**i18n** — Title and description keys are updated in both `en` and `fr` locales. No new key names needed; the existing `tour.seal.*` keys are repurposed with the richer spec copy. The existing `tour.registry.*` keys (already present but unused) are wired to the new supply-chain step.

**LocalStorage persistence** — `connectionStore` initialises `lastMfaTimestamp` from `localStorage` at store creation time and writes back on every `setLastMfaTimestamp` call. Storage access is wrapped in try/catch so SSR environments and locked storage contexts degrade gracefully. The in-memory value always wins for the current session.

**XSS audit** — Both `TachyonTopologyPanel` and `TachyonUsersPanel` already route all dynamic data through their own `escape()` helpers before any `innerHTML` assignment. No changes needed.

### 2. Cross-Layer Validation Script

**Strategy** — Pure static grep; no compilation required. This makes it cheap to run early in CI (before the multi-minute Cargo build) and keeps the script dependency-free.

**Endpoint matching** — `sed` extracts `(constant_name, path_value)` pairs from `const ADMIN_*_PATH` declarations in `tachyon-client/src/lib.rs`. For each pair the script checks that the literal path (or a prefix with a `/` continuation for parameterised routes like `/admin/iam/users/{username}`) appears in `core-host/src/host_core/app_runtime.rs`. Exact match or prefix-slash match covers both static and parameterised Axum routes.

**Stronghold guard** — If `tauri-plugin-stronghold` is present in `tachyon-ui/Cargo.toml`, the script asserts at least one `Stronghold::` call exists in `tachyon-ui/src/main.rs`. This prevents the "declared but dead" regression identified in the audit.

**CI placement** — The step runs in `rust-ci` immediately before "Build guest artifacts". Failing fast before the Cargo build saves ~5 minutes of CI time per broken PR.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Tour step position | After registry | Before overview | User needs context before seeing the sign-off flow |
| localStorage wrapping | try/catch | Raw access | Tauri WebView can throw on storage in hardened contexts |
| Validation mechanism | bash + sed/grep | cargo test or TS script | Zero new dependencies; runs before Rust toolchain warms up |
| Axum route matching | Exact + prefix-slash | Regex | Simpler; all parameterised routes use `{param}` so prefix match is unambiguous |
