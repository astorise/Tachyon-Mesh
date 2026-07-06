## Why

`IntegrityConfig.require_scopes` defaults to `false` by design: the field's own doc comment and a regression test (`routes_without_scopes_pass_validation_under_default_require_scopes_false`) say the default may only flip "via a separate openspec change once telemetry shows zero allow-all deployments." That telemetry doesn't exist yet — no fleet has run the migration tooling (`tachyon_suggest_scopes`, `tachyon_set_route_scopes`, `tachyon_get_scope_denials`, the ScopesPanel UI) long enough to prove it. GitHub issue #311 asks us to own that eventual flip. This change is the "separate openspec change" the gate refers to: it stakes out the decision criteria and rollout plan now, and lands the parts of the migration that don't require fleet telemetry to be safe — an actionable rejection error and a documented legacy opt-out — without touching the default itself.

## What Changes

- Add the missing spec coverage for the `require_scopes` node-level flag and its Phase 3 opt-in behavior to `faas-import-scoping` (this was implemented in the `faas-wit-import-scoping` change but never given its own spec requirement).
- Make the `require_scopes: true` rejection errors in `validate_require_scopes` (`core-host/src/host_core/integrity_config.rs`) actionable: name `tachyon_suggest_scopes` as the remediation path for both the "missing scopes block" and "resolves to allow-all" cases.
- Document an explicit legacy opt-out: operators who complete Phase 2 tightening but are not ready for a fleet-wide strict default may pin `require_scopes: false` in their own manifest to keep prior behavior even after the fleet default changes in a future change.
- Record the observation-window decision criteria (what "zero allow-all deployments" means operationally, how long the window runs, who signs off) that gate the actual default flip.
- **Not in this change**: flipping `require_scopes`'s default to `true`, and migrating the remaining allow-all system routes (`/metrics`, `/system/logger`) in the reference `integrity.lock`. Both require evidence or verification this change doesn't have — real fleet telemetry for the former, a rebuilt-and-relinked guest check plus manifest re-signing for the latter (`scripts/seal-guest-openai.js` shows re-signing isn't a trivial JSON edit). Both are tracked as blocked prerequisite tasks.

## Capabilities

### New Capabilities

_None._

### Modified Capabilities

- `faas-import-scoping`: adds the previously-unspecified `require_scopes` node-flag requirement (Phase 3 opt-in strict validation), makes its rejection errors name the remediation tool, and adds the legacy-opt-out guarantee for the eventual Phase 4 default change.

## Impact

- **Code**: `core-host/src/host_core/integrity_config.rs` (`validate_require_scopes` error strings only — no behavior change to what is accepted/rejected); `core-host/src/host_core/tests/config_validation.rs` (assert the new error text).
- **Docs**: `docs/faas-import-scoping.md` Phase 4 section gains the legacy opt-out note.
- **Specs**: `openspec/specs/faas-import-scoping/spec.md` gains a requirement documenting `require_scopes` and its error-message contract.
- **No API/wire/manifest-schema change**: `require_scopes` already exists and already defaults to `false`; this change only improves messaging and documentation around it.
- **Blocked follow-up work** (tracked in `tasks.md`, not executed here): running the telemetry observation window, migrating `/metrics` and `/system/logger` in `integrity.lock` to explicit scopes, and the actual default flip PR once the observation window closes clean.
