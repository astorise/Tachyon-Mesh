## Context

This change is the documentation-closure output of a full OpenSpec coverage
audit (and a follow-up re-audit after the discrepancy fixes landed). The audit
inventoried every code surface (37 WASM systems, `core-host`, four sibling
crates, 38 WIT files, the Tachyon-UI web components, the MCP tool set, and the
polyglot SDKs) and matched each against `openspec/specs/`, the active change, and
the archive. Coverage was ~98%; most apparent gaps turned out to be specs filed
under a different name — for example `system-faas-gc` is documented by
`volume-garbage-collector`, the gitops config store by `distributed-control-plane`
("GitOps Multi-branching"), and the streaming WIT by the in-flight
`openai-inference-fidelity` change.

## Decision

Document the genuinely-uncovered behaviors exactly as the code behaves today,
rather than specifying an aspirational target. These are stable, shipped surfaces.

- **prom** is modeled as its own capability — consistent with the existing
  `system-faas-openapi` and `system-faas-config-api` component capabilities —
  rather than folded into `faas-observability`.
- **upload_model** extends `mcp-server`, matching the one-requirement-per-tool
  style already used for `register_resource` and the lifecycle tools.
- **dist-limiter** extends `distributed-crdt-rate-limiter` with the observable
  HTTP/merge contract, leaving the existing intent requirements intact.
- **TEE** extends `confidential-computing-tee` with the concrete backend contract.
  The `LocalEnclave` mode is documented honestly as a non-confidential, local /
  development path (it runs on the standard engine and only annotates the
  response); the hardware-confidentiality guarantee applies only to the Enarx
  Keep path. This avoids the spec over-claiming hardware isolation for a mode that
  provides none.
- **metering** modifies the existing `tracing-metering` requirement so the host
  exporter — not `system-faas-metering` — owns the in-memory aggregation and the
  size/interval flush, and adds a requirement for the durable `metering_outbox`
  staging. The new requirements claim only what the code guarantees: durable
  staging before export, deletion on success, and retention on failure. They do
  **not** claim automatic re-export of retained entries (see follow-up below).

## Discrepancies fixed in PR #141

The audit's first pass flagged four places where an existing requirement
described behavior the code did not implement. These were filed as code defects
(issues #135–#138) and fixed in PR #141:

1. **cdc-broadcaster status** (#135) — now returns `401` (was `403`). The
   `remediation-plan` spec was already correct; no doc change needed.
2. **buffer replay header** (#136) — now emits `x-tachyon-buffered` with the
   serving tier. The `adaptive-pressure-control-and-tiered-buffering` spec was
   already correct; no doc change needed.
3. **metering flush** (#137) — a host-side exporter now batches and flushes on a
   60 s interval. This introduced the exporter-ownership and durable-outbox
   surfaces documented by the `tracing-metering` delta in this change.
4. **TEE delegation** (#138) — `requires_tee` routes now delegate to the
   configured backend. This introduced the backend contract documented by the
   `confidential-computing-tee` delta in this change.

## Follow-up not covered here

- **Metering outbox drain/retry is not wired.** Retained `metering_outbox`
  entries are durable but there is currently no sweeper that re-exports them
  (unlike `AuthzPurgeOutbox` and `ConfigUpdateOutbox`, which have steady-cadence
  drains in `supervisors.rs`). The `tracing-metering` delta therefore documents
  durability and retention only, not automatic recovery. Wiring a drain is a
  code follow-up, out of scope for this documentation change.

## Notes

Two minor invariants are intentionally left undocumented here because they are
hardening details rather than capability contracts: `system-faas-tde` refuses to
operate without a valid 256-bit `TDE_KEY_HEX`, and `system-faas-metering`
normalizes `lora_adapter_load` events into a `meter_kind: "fuel"` record. The
dormant `wit/mesh/routing.wit` `get-primary-node` interface (defined but imported
by no component) is also left undocumented pending its activation or removal.
