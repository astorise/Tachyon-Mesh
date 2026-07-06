## Context

`faas-import-scoping` (archived change `2026-05-24-faas-wit-import-scoping`) shipped a four-phase migration for per-deployment WIT import scoping:

1. Phase 1 — code lands, default `allow-all`.
2. Phase 2 — operators tighten manifests, guided by `faas_scopes_allow_all_total` / `faas_scope_denials_total`.
3. Phase 3 — opt-in strict default: node sets `require_scopes: true`, rejecting allow-all manifests at submission.
4. Phase 4 — flip the fleet-wide default to `true`, "behind its own openspec change" (the design doc's words).

Phases 1–3 are implemented (`core-host/src/host_core/integrity_config.rs::validate_require_scopes`, the `require_scopes` field in `domain_types.rs`, the MCP tools `tachyon_suggest_scopes` / `tachyon_set_route_scopes` / `tachyon_get_scope_denials`, and the `ScopesPanel` UI). Phase 4 was deliberately left undone, gated on telemetry showing zero allow-all deployments fleet-wide. GitHub issue #311 is the tracking issue for that flip. No fleet has run Phase 2/3 long enough to produce that telemetry, so this change cannot execute Phase 4 — it can only own the decision and land the parts of the migration that are safe without fleet evidence.

## Goals / Non-Goals

**Goals:**

- Give `faas-import-scoping` the spec coverage it's missing for the `require_scopes` node flag (Phase 3), so the eventual Phase 4 delta has something to modify instead of inventing the requirement from scratch.
- Make the Phase 3 rejection errors actionable: an operator who turns on `require_scopes` and gets rejected should be told which tool fixes it, not just what's wrong.
- Give operators a documented, permanent way to opt out of the future fleet default (`require_scopes: false` explicit in their own manifest) so Phase 4 is not a forced migration for clusters with a legitimate reason to stay on allow-all (e.g., air-gapped clusters running a fixed, audited manifest).
- Write down the decision criteria for Phase 4 so it's a mechanical go/no-go check for whoever opens that change, not a judgment call made from scratch.

**Non-Goals:**

- Flipping `require_scopes`'s default. That requires real fleet telemetry this change cannot manufacture.
- Migrating `/metrics` and `/system/logger` in the reference `integrity.lock` off allow-all. `system-faas-guest` (the world both routes target) imports `storage-broker` and `outbound-http` unconditionally; whether an empty `scopes: {}` block (as opposed to omitting `scopes` entirely) breaks their link depends on whether wit-bindgen's generated import stubs survive dead-code elimination when unused — that's a build-and-link question, not a JSON edit, and `integrity.lock` is a *signed* manifest (re-signing needs a sealing script akin to `scripts/seal-guest-openai.js`, which doesn't exist yet for these routes). Verifying this safely is out of scope here.
- Running the observation window itself. That's an operations activity that happens after this change merges, not a code change.

## Decisions

### D1. Phase 4 go/no-go criteria (written down now, checked later)

**Decision.** Phase 4 (the actual default flip) may proceed once, for a continuous window of at least 2 weeks:
- `faas_scopes_allow_all_total` is flat (no increment) across every deployment in the fleet, **and**
- `tachyon_get_scope_denials` reports `allow_all: false` for every route on every node reachable from the MCP server, **and**
- no node has `require_scopes: false` set *without* an accompanying documented reason (see D3) — an unexplained `false` is a signal Phase 2 isn't finished, not a legacy opt-out.

**Why.** The original design left "telemetry shows zero allow-all deployments" undefined. Without a written threshold, whoever eventually opens the Phase 4 change has to reconstruct intent. Fixing the criteria now, while the rationale is fresh, turns that into a checklist.

**Alternatives rejected.** A percentage threshold (e.g., "99% of deployments scoped") was considered and rejected: allow-all is an authorization bypass, not a performance metric — a single unscoped deployment in a multi-tenant cluster is a real gap, not noise to average away.

### D2. Actionable error messages name the remediation tool

**Decision.** `validate_require_scopes` (`core-host/src/host_core/integrity_config.rs:1034`) keeps its existing substrings (`scopes`, `require_scopes`, `allow-all`) so `config_validation.rs` assertions still pass, and appends a clause naming `tachyon_suggest_scopes` as the remediation path in both error branches (missing `scopes` block; resolves to allow-all).

**Why.** This is the one piece of the migration path that doesn't need fleet telemetry to be worth doing today — every operator who already opts into Phase 3 (`require_scopes: true`) hits these errors now, and the fix already exists (`tachyon_suggest_scopes`) but isn't discoverable from the error text.

**Alternatives rejected.** A structured error type carrying a `remediation` field was considered; rejected as overkill for a `Result<(), anyhow::Error>` validation path with no existing structured-error convention elsewhere in this file.

### D3. Legacy opt-out is a first-class, permanent state — not a deprecation grace period

**Decision.** After Phase 4 ships (in a future change), `require_scopes: false` remains a fully supported, explicit manifest setting — not a deprecated flag scheduled for removal. Document it as the answer for clusters that cannot or will not scope every deployment (fixed-manifest air-gapped clusters, single-tenant nodes where the authorization boundary genuinely doesn't matter).

**Why.** Phase 4 changes the *default*, not the *option*. Conflating "we're changing what happens when you say nothing" with "we're taking away your ability to say `false`" would turn a safe default-flip into a forced migration, which is a much larger and riskier change than issue #311 asks for.

### D4. Retroactively spec the `require_scopes` flag under `faas-import-scoping`

**Decision.** Add the missing requirement to `openspec/specs/faas-import-scoping/spec.md` describing current Phase 3 behavior (the flag, its default, what it rejects) as a `Modified Capability` delta in this change, even though the code already exists. Do not backdate it into the archived `2026-05-24-faas-wit-import-scoping` change.

**Why.** `tasks.md` in that archived change (item 8.2) implemented the flag, but no requirement was ever added to `spec.md` — a documentation gap, not a behavior gap. This change is the natural place to close it because it's already touching the same requirement area (the rejection error text) and because Phase 4's future delta needs an existing requirement to modify rather than introducing `require_scopes` from nothing.

### D5. Do not touch `integrity.lock` in this change

**Decision.** Leave `/metrics` and `/system/logger` on allow-all in the reference manifest. Track migrating them as a blocked prerequisite task with the link-time question spelled out, rather than guessing at an answer and shipping an unverified change to a signed artifact.

**Why.** See Non-Goals — this needs a build/link verification cycle and a re-signing step this change doesn't have the tooling for. Shipping a guess here risks breaking the reference manifest CI relies on, which is a worse outcome than leaving the task open.

## Risks / Trade-offs

- **[Risk]** Writing down Phase 4 criteria (D1) now might not match whatever operational reality looks like when a fleet actually exists to measure. → **Mitigation.** The criteria live in this change's design doc, not in code — the future Phase 4 change can revise them with a one-line rationale if reality doesn't match, rather than starting from zero.
- **[Risk]** Someone reads "dedicated openspec change" in issue #311 and expects the default to actually flip here. → **Mitigation.** proposal.md's "Not in this change" section and this design's Non-Goals both say so explicitly; the PR description will link back to both.
- **[Trade-off]** Deferring the `integrity.lock` migration means the reference manifest keeps two allow-all routes indefinitely until someone does the link-time verification work. Acceptable: `/metrics` and `/system/logger` are host-internal system routes, not multi-tenant guest deployments — the authorization gap they represent is much smaller than an unscoped user-facing FaaS route.

## Migration Plan

This change performs no runtime migration — Phases 1–3 are already live, Phase 4 is deferred. Rollback, if needed, is simply reverting the error-message and doc changes; no manifest, wire, or schema format changes are involved.

## Open Questions

- **OQ1.** Who signs off that the D1 criteria are met — an individual operator per cluster, or a fleet-wide review before the Phase 4 PR is opened? **Proposed default: whoever opens the Phase 4 change includes the observation-window data in the PR description; standard review applies.** Left open for the Phase 4 change to confirm.
- **OQ2.** Should `tachyon_get_scope_denials` gain a fleet-wide rollup (today it's per-node) to make the D1 check a single query instead of one per node? Worth doing before Phase 4 if the fleet is large enough that per-node checks are impractical. Deferred — not needed for this change.
