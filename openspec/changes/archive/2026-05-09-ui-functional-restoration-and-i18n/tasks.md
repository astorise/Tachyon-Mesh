# Implementation Tasks

## 1. Enrollment Flow Cleanup
- [x] 1.1 Remove the redundant `iam-signup-form` block, its bindings, and the
       `stageOperator()` method from `<tachyon-iam>`.
- [x] 1.2 Drop the dead "no-MFA" branch in `login()`; rely on the staged
       login + TOTP finalization path that the backend always returns.
- [x] 1.3 Synchronize the remaining node-URL fields without depending on the
       removed `iam-url` input.

## 2. Real Step-up MFA
- [x] 2.1 Replace `verify_session_totp` with a command that consumes the
       persisted operator profile, calls `tachyon-client::authn_login`, then
       `finalize_login` with the supplied code, returning success only on
       backend acceptance.
- [x] 2.2 Surface a clear error when no remembered profile is available so
       the operator knows step-up cannot succeed without a credential
       refresh.

## 3. Live Telemetry Wiring
- [x] 3.1 Expose `get_metrics`, `tail_logs`, and `get_shadow_diffs` as Tauri
       commands.
- [x] 3.2 Update `<tachyon-overview-panel>` so the visible "Active Edge
       Nodes", "Global Wasm Instances", and "AI/GPU Utilization" values are
       derived from `RuntimeMetrics` (queue depth, latency, error rate) and
       fall back to the mesh graph only when no remote endpoint is reachable.
- [x] 3.3 Add a runtime-metrics + recent-logs + shadow-divergence section to
       `<tachyon-observability-panel>` that consumes the new commands.

## 4. Localization Sweep
- [x] 4.1 Extend `utils/i18n.ts` with the `iam.*`, `mfa.*`,
       `observability.*`, `routing.*`, and `storage.*` keys used by the
       refactored components for both `en` and `fr`.
- [x] 4.2 Refactor `<tachyon-iam>` and `<tachyon-mfa-prompt>` to render every
       operator-visible string through `t(...)`.
- [x] 4.3 Refactor the Observability and Storage panels' new live sections
       through `t(...)` so the language toggle has visible effect outside the
       shell chrome.

## 5. Component Registry Hygiene
- [x] 5.1 Remove the `topology` route and the `TachyonTopologyPanel`
       fake-class alias until a real topology component exists.
- [x] 5.2 Drop the duplicate `registry` route; expose the asset registry
       only through the `supply-chain` route.
- [x] 5.3 Update the guided tour copy that referenced the registry route.

## 6. Current State Previews
- [x] 6.1 Add a sealed-routes preview to `<tachyon-routing-panel>` using
       `get_mesh_graph`.
- [x] 6.2 Add a workspace-resources preview to `<tachyon-storage-panel>`
       using `read_resources`.

## 7. Documentation
- [x] 7.1 Update the affected capability specs (`iam-webcomponent`,
       `global-overview`, `compute-observability`) with the new requirements
       under this change.
