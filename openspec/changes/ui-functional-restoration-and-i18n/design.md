# Design Notes

## Step-up MFA without a new backend endpoint

The authn WIT (`wit/authn.wit`) does not yet declare a standalone
`verify-totp` function. Adding one would touch the WIT contract, the
`system-faas-authn` guest, and the SDK SemVer surface. To stay within the
scope of a UI restoration change, we reuse the existing
`stage-login` + `finalize-login` pair:

1. The Tauri shell reads the persisted operator profile from the secure
   store (Stronghold-backed JSON file) and calls
   `tachyon-client::authn_login` with the remembered URL, username, password,
   and custom CA.
2. The backend issues a fresh `staged-login-session`, exactly as if the
   operator had just submitted the login form.
3. The shell calls `tachyon-client::finalize_login` with the staged session
   identifier and the TOTP code the operator typed in
   `<tachyon-mfa-prompt>`. A successful finalization is the proof that the
   operator possesses the TOTP secret bound to that account.

If `remember credentials` is disabled, step-up cannot succeed — we surface
that explicitly to the operator instead of pretending the prompt validated
anything. A future change can replace this with a dedicated
`verify-totp` WIT function once the SDK SemVer impact is acceptable.

## Overview metrics composition

`<tachyon-overview-panel>` keeps its three-card layout but the card values
now blend two data sources:

| Card | Source |
| --- | --- |
| Active Edge Nodes | `MeshGraphSnapshot.batch_targets.length` (sealed config; the host does not yet expose a peer-list endpoint) |
| Global Wasm Instances | `RuntimeMetrics.queue_depth` (live admit pressure) |
| AI/GPU Utilization | derived from `RuntimeMetrics.error_rate` and `p99_latency_ms` clamped to `[0, 100]` |

When the host is reachable, the `RuntimeMetrics` source string replaces the
"Mesh telemetry online" status badge; otherwise we fall back to the existing
sealed-config message. The intent is to give the operator real numbers that
move when the cluster is busy, rather than constants computed from the
sealed file.

## Observability live panels

The observability dashboard keeps its OTLP form but gains three additional
sections rendered above it:

- **Runtime metrics** — error rate %, p50/p99 latency, queue depth, sourced
  from `get_metrics`.
- **Recent log lines** — last 50 entries from `tail_logs`, fixed-width font.
- **Shadow divergences** — entries from `get_shadow_diffs`, with the
  primary/shadow status code pair when present.

Each section degrades gracefully when the host is offline (empty list or
zero-valued metrics), to match the resilience contract that
`resilientInvoke` already enforces.

## Routing & Storage current-state previews

Both panels now pull a snapshot before rendering the configuration form:

- Routing reads `get_mesh_graph` and lists the sealed routes (name, path,
  target count, TEE flag).
- Storage reads `read_resources` and lists the workspace overlay resources
  (name, type, target, pending flag).

These previews are read-only; the form below them remains the only way to
mutate state.

## Component registry cleanup

The `topology` placeholder is removed from `ComponentRegistry.ts` and
`TachyonTopologyPanel` is deleted from `TachyonRoutingDashboard.ts`. The
`registry` and `supply-chain` routes deduplicate to a single `supply-chain`
entry; the guided tour copy that referenced "registry" is rewritten to point
at the unified route.
