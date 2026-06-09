## Context

This change is the documentation-closure output of a full OpenSpec coverage
audit. The audit inventoried every code surface (37 WASM systems, `core-host`,
four sibling crates, 38 WIT files, the Tachyon-UI web components, the MCP tool
set, and the polyglot SDKs) and matched each against `openspec/specs/`, the
active change, and the archive. Coverage was ~98%; most apparent gaps turned out
to be specs filed under a different name — for example `system-faas-gc` is
documented by `volume-garbage-collector`, the gitops config store by
`distributed-control-plane` ("GitOps Multi-branching"), and the streaming WIT
(`response-body`, `compute-stream`/`token-stream`) by the in-flight
`openai-inference-fidelity` change.

## Decision

Document the three genuinely-uncovered behaviors exactly as the code behaves
today, rather than specifying an aspirational target. These are stable, shipped
surfaces, so the spec should describe what is.

- **prom** is modeled as its own capability — consistent with the existing
  `system-faas-openapi` and `system-faas-config-api` component capabilities —
  rather than folded into `faas-observability`, because it is a distinct
  deployable component with its own output contract.
- **upload_model** extends `mcp-server`, matching the one-requirement-per-tool
  style already used for `register_resource`, `list_resources`, and the lifecycle
  tools.
- **dist-limiter** extends `distributed-crdt-rate-limiter` with the observable
  HTTP/merge contract, leaving the existing intent requirements intact.

## Discrepancies handed off as code defects

The audit also found four places where an existing requirement describes
behavior the code does not implement. These are treated as code defects to fix
(the code should converge to the spec), not as docs to rewrite, and are recorded
here so the trail is durable:

1. **metering batch semantics.** `tracing-metering` requires
   `system-faas-metering` to batch `tachyon.telemetry.usage` events in memory and
   flush on a periodic interval (default 60 s). The component instead appends each
   POSTed batch synchronously to `/app/data/metering.ndjson` with no in-memory
   accumulation and no interval flush.

2. **TEE enclave delegation.** `confidential-computing-tee` requires `core-host`
   to bypass the pooled Wasmtime engine and delegate `requires_tee` modules to a
   hardware enclave backend. The code parses and validates the `requires_tee`
   flag but has no enclave backend — the delegation is unimplemented.

3. **cdc-broadcaster status code.** `remediation-plan` mandates HTTP `401` for the
   fail-closed rejection until a real Biscuit verifier is wired in.
   `system-faas-cdc-broadcaster` returns `403`.

4. **buffer replay header.** `adaptive-pressure-control-and-tiered-buffering`
   specifies the response header `x-tachyon-buffered` naming the buffering tier.
   `system-faas-buffer` emits `x-tachyon-buffer-replay` / `x-tachyon-job-id`
   instead.

## Notes

Two minor invariants are intentionally left undocumented here because they are
hardening details rather than capability contracts: `system-faas-tde` refuses to
operate without a valid 256-bit `TDE_KEY_HEX`, and `system-faas-metering`
normalizes `lora_adapter_load` events into a `meter_kind: "fuel"` record. The
dormant `wit/mesh/routing.wit` `get-primary-node` interface (defined but imported
by no component) is also left undocumented pending its activation or removal.
