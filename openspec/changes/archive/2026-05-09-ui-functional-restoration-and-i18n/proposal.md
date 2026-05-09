# Proposal: UI Functional Restoration and Localization Sweep

## 1. Context

A retrospective audit of `tachyon-ui` revealed that several previously-archived
UI changes were merged with their tasks marked complete, but the resulting
behaviour does not match the operator's expectations:

1. **Enrollment is broken.** `<tachyon-iam>` ships with two competing signup
   forms in the same Shadow DOM. The legacy `iam-signup-form` calls
   `stage_signup` directly, skipping invite-token validation and the TOTP
   finalization step, so the operator sees a "success" toast but is never
   actually authenticated.
2. **Step-up MFA is cosmetic.** The Tauri command `verify_session_totp` only
   checks that the supplied code is six digits. Any value such as `000000`
   currently unlocks `seal_and_apply_manifest`, contradicting the
   `complete-auth-flow` requirement that step-up MFA gate sensitive writes.
3. **Overview hides real telemetry.** `<tachyon-overview-panel>` renders
   metrics that are derived from the local sealed configuration
   (`integrity.lock`) rather than from the live `/admin/metrics` endpoint that
   `tachyon-client::get_metrics` already implements. The active edge nodes,
   wasm instances, and GPU utilization values are therefore not real
   measurements.
4. **Observability panel is write-only.** The dashboard exposes an OTLP
   configuration form but never surfaces the runtime metrics, recent log
   lines, or shadow-traffic divergences that the backend already exposes via
   `/admin/metrics`, `/admin/logs`, and `/admin/shadow/diffs`.
5. **Localization is incomplete.** Only `TachyonAppShell`, the guided tour,
   and `TachyonOverviewPanel` consume `utils/i18n.ts`. The IAM overlay, the
   MFA step-up dialog, and every domain panel hardcode English strings, so
   switching the language toggle to French has no effect outside the shell
   chrome.
6. **Component registry is inconsistent.** The `topology` route resolves to a
   class that simply re-exports the routing dashboard, and `registry` and
   `supply-chain` both bind to the same `<tachyon-supply-chain-panel>`,
   producing duplicate menu entries that point at the same view.
7. **Domain dashboards are write-only.** Routing, Storage, and similar panels
   only present a configuration form. Operators cannot see what is currently
   deployed before staging a change, which makes the UI feel like an empty
   shell.

## 2. Solution

Restore the UI to a functional state by:

- Collapsing `<tachyon-iam>` to a single, correct enrollment flow
  (validate-token → profile → TOTP) and dropping the dead post-MFA "no-MFA"
  branch.
- Reconnecting `verify_session_totp` to the existing staged-login pipeline so
  step-up actually validates the operator's TOTP secret before unlocking
  sensitive writes.
- Exposing `get_metrics`, `tail_logs`, and `get_shadow_diffs` as Tauri
  commands and consuming them from `<tachyon-overview-panel>` and
  `<tachyon-observability-panel>`.
- Extending `utils/i18n.ts` with the strings needed by `<tachyon-iam>`,
  `<tachyon-mfa-prompt>`, and the live observability panels, and routing
  every operator-visible string through the `t(...)` helper for those
  components.
- Cleaning the route registry: removing the topology placeholder until a real
  topology panel exists, deduplicating the asset-registry / supply-chain
  entries.
- Adding a "current deployed state" preview to the Routing and Storage
  dashboards using `read_resources` and `get_mesh_graph`, so operators can
  inspect the sealed state before staging modifications.

## 3. Non-goals

- Building a full mesh topology visualization (deferred until the supporting
  data plane is in place).
- Translating every domain dashboard into French. The sweep adds the
  affordances and translates the IAM, MFA, observability, overview, and shell
  surfaces; remaining panels keep their English source strings until a
  follow-up sweep adds the dictionaries.
- Replacing the existing chaos-engineering UI surface (none exists today;
  exposing `run_chaos_scenario` is out of scope here).
