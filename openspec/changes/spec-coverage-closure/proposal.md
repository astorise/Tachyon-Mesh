## Why

An OpenSpec coverage audit cross-referenced every shipped code surface — the 37
WASM system components, `core-host`, the sibling crates, the WIT contracts, the
Tachyon-UI web components, the MCP tools, and the SDKs — against the 137
capability specs, the active change, and the 259 archived changes. Coverage was
~98%: nearly every module, interface, panel, and tool already maps to a
governing requirement. Three shipped, observable behaviors were the exception —
they have no requirement anywhere:

- **`system-faas-prom`** renders a Prometheus text exposition on every request,
  but no spec pins the metric set or its format. The privileged telemetry-read
  mechanism it consumes is specified (`faas-observability`); the exposition
  contract it produces is not.
- **`tachyon_upload_model`** is a registered, rate-limited MCP tool, yet the MCP
  spec enumerates every other lifecycle tool and omits this one.
- **`system-faas-dist-limiter`** has a documented *intent* (identity-scoped CRDT
  counters, bounded fail-open) but its observable inter-node contract — the
  `/check`, `/merge`, `/state` HTTP surface, the windowed keying, and the
  G-counter convergence rule — is unspecified.

This change documents the existing behavior as it ships so the specs match
reality. It changes no code. Four separate spec↔code *discrepancies* surfaced by
the same audit are handled as code defects rather than doc edits and are listed
in `design.md` for follow-up.

## What Changes

- **`system-faas-prom` exposition contract (new capability).** Specifies that
  the component reads the host telemetry snapshot through the privileged reader
  world and renders a fixed set of nine `tachyon_*` series as Prometheus text
  with `# TYPE` lines, returning `200` for any request.
- **`tachyon_upload_model` MCP tool (`mcp-server`).** Specifies the tool name,
  the required `path` argument, delegation to
  `tachyon_client::push_large_model`, the registered tool schema, and the tight
  per-tool rate-limit budget shared with other large mutators.
- **Distributed limiter sync surface (`distributed-crdt-rate-limiter`).**
  Specifies the `/check` / `/merge` / `/state` endpoints, the `{key}:{window}`
  time-windowed keying derived from `DIST_LIMIT_WINDOW_SECONDS`, and the
  per-(key,node) maxima merge that makes the G-counter convergent.

## Capabilities

### New Capabilities
- `system-faas-prom`: the Prometheus exposition contract served by the
  `system-faas-prom` WASM component.

### Modified Capabilities
- `mcp-server`: adds the `tachyon_upload_model` lifecycle-tool requirement.
- `distributed-crdt-rate-limiter`: adds the limiter's HTTP sync surface,
  time-window keying, and merge-convergence requirements (existing intent
  requirements are unchanged).

## Impact

- Documentation-only: no code, dependency, WIT, or interface change. The three
  behaviors already ship in `systems/system-faas-prom/src/lib.rs`,
  `tachyon-mcp/src/main.rs`, and `systems/system-faas-dist-limiter/src/lib.rs`.
- Specs affected: new `system-faas-prom`, plus deltas to `mcp-server` and
  `distributed-crdt-rate-limiter`.
- Out of scope, tracked separately as code defects (see `design.md`): metering
  batch semantics, TEE enclave delegation, the cdc-broadcaster status code, and
  the buffer replay header — each a case where an existing requirement describes
  behavior the code does not implement.
