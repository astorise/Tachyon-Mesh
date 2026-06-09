## Why

An OpenSpec coverage audit cross-referenced every shipped code surface — the 37
WASM system components, `core-host`, the sibling crates, the WIT contracts, the
Tachyon-UI web components, the MCP tools, and the SDKs — against the capability
specs, the active change, and the archived changes. Coverage was ~98%. This
change documents the behaviors that had no governing requirement.

The first pass found three shipped surfaces with no requirement anywhere:

- **`system-faas-prom`** renders a Prometheus text exposition on every request,
  but no spec pins the metric set or its format.
- **`tachyon_upload_model`** is a registered, rate-limited MCP tool, yet the MCP
  spec enumerates every other lifecycle tool and omits this one.
- **`system-faas-dist-limiter`** has a documented *intent* (identity-scoped CRDT
  counters, bounded fail-open) but its observable inter-node contract — the
  `/check`, `/merge`, `/state` HTTP surface, the windowed keying, and the
  G-counter convergence rule — is unspecified.

A follow-up pass after the discrepancy fixes landed (PR #141 closed issues
#135–#138) found that fixing two of them introduced new observable surfaces that
are themselves undocumented:

- **TEE delegation** is now implemented: `requires_tee` routes dispatch to a
  configured `tee_backend` (`LocalEnclave` or `Enarx { keep_endpoint }`) with a
  Keep invocation protocol and `x-tachyon-runtime` / `x-tachyon-tee-backend`
  response annotations. The spec only described delegation at a high level.
- **Host metering exporter + durable outbox**: the host exporter now owns the
  in-memory aggregation and the periodic (60 s) / batch-size flush, staging each
  batch in a durable `metering_outbox` before forwarding to `system-faas-metering`.
  The `tracing-metering` requirement still attributed in-memory batching to
  `system-faas-metering`.

This change documents all of the above as the code behaves today. It changes no
code.

## What Changes

- **`system-faas-prom` exposition contract (new capability).** Specifies the
  privileged telemetry read and the fixed set of nine `tachyon_*` series rendered
  as Prometheus text with `# TYPE` lines, returning `200` for any request.
- **`tachyon_upload_model` MCP tool (`mcp-server`).** Specifies the tool name,
  the required `path` argument, delegation to `tachyon_client::push_large_model`,
  the registered schema, and the rate-limit budget.
- **Distributed limiter sync surface (`distributed-crdt-rate-limiter`).**
  Specifies the `/check` / `/merge` / `/state` endpoints, the `{key}:{window}`
  time-windowed keying, and the per-(key,node) maxima merge.
- **TEE backend contract (`confidential-computing-tee`).** Specifies backend
  selection and the `503` fail-closed path, the `LocalEnclave` (no hardware
  isolation) vs `Enarx` Keep distinction, the Keep invocation protocol, and the
  TEE response-annotation headers.
- **Metering exporter ownership + durable outbox (`tracing-metering`).**
  Realigns the existing requirement so the host exporter owns the in-memory
  aggregation and size/interval flush, and adds the durable `metering_outbox`
  staging (persist-before-export, delete-on-success, retain-on-failure).

## Capabilities

### New Capabilities
- `system-faas-prom`: the Prometheus exposition contract served by the
  `system-faas-prom` WASM component.

### Modified Capabilities
- `mcp-server`: adds the `tachyon_upload_model` lifecycle-tool requirement.
- `distributed-crdt-rate-limiter`: adds the limiter's HTTP sync surface,
  time-window keying, and merge-convergence requirements.
- `confidential-computing-tee`: adds the concrete backend-selection, Enarx Keep
  protocol, and response-annotation requirements.
- `tracing-metering`: realigns the metering-aggregation ownership to the host
  exporter and adds the durable metering-outbox requirement.

## Impact

- Documentation-only: no code, dependency, WIT, or interface change. The behaviors
  ship in `systems/system-faas-prom/src/lib.rs`, `tachyon-mcp/src/main.rs`,
  `systems/system-faas-dist-limiter/src/lib.rs`, and — after PR #141 —
  `core-host/src/host_core/app_runtime.rs` (TEE delegation + metering exporter),
  `core-host/src/host_core/constants.rs`, and `core-host/src/store/mod.rs`
  (`MeteringOutbox`).
- Specs affected: new `system-faas-prom`; deltas to `mcp-server`,
  `distributed-crdt-rate-limiter`, `confidential-computing-tee`, and
  `tracing-metering`.
- The four spec↔code discrepancies the audit originally flagged
  (cdc-broadcaster `401`, buffer `x-tachyon-buffered`, metering flush, TEE
  delegation) were fixed in code by PR #141; the cdc-broadcaster and buffer fixes
  brought the code into line with specs that were already correct, so they need no
  new documentation here.
