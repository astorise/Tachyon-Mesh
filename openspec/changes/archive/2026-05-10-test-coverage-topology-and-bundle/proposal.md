# Proposal: Test Coverage — Topology Helpers and Bundle Handler

## Why

The topology canvas interactions and the smart deployment bundle handler were
delivered without automated tests. Two gaps needed closing:

1. **Frontend** — `topology.helpers.ts` contains pure functions
   (`themeFor`, `badgeValueFor`, `serializeGraph`, `filterGraphOnDelete`,
   `clampPosition`) that are called on every render cycle. They had no test
   harness.

2. **Backend** — `parse_bundle_manifest_yaml` and
   `admin_manifest_bundle_handler` handle the operator deployment path. The
   YAML parser is a hand-written state machine; the handler coordinates trust
   injection, SemVer conflict detection, and Ed25519 signing. Both were
   uncovered.

A secondary fix: `admin_manifest_update_handler` was using
`verify_integrity_payload` (which only accepts the embedded boot key) instead
of forwarding the runtime's `trusted_signers` to
`verify_integrity_payload_with_trusted`. This meant cluster nodes that had been
bootstrapped into `trusted_signers` via the bundle flow could not then push
plain manifest updates — only the embedded key was accepted.

## What Changes

### Frontend (`tachyon-ui`)

- `vite.config.ts` — adds `setupFiles: ["./src/test-setup.ts"]` so the
  `localStorage` stub is wired before every test file runs.
- `src/test-setup.ts` — provides a consistent `localStorage` mock and a
  `CustomEvent` stub for happy-dom environments.
- `src/components/domains/topology.helpers.ts` — `parse_bundle_manifest_yaml`
  and `BundleManifestFields` promoted to `pub(crate)` to enable white-box
  tests.
- `src/components/domains/topology.helpers.test.ts` — 27 tests covering all
  five exported helper functions (all 8 node types × badgeValueFor, theme
  consistency, graph mutations, position clamping).
- `src/utils/i18n.test.ts` — 17 tests covering `t()`, `getLanguage` /
  `setLanguage`, `ErrorTranslator`, and `translateBackendError`.

### Backend (`core-host`)

- `integrity_config.rs`
  - `parse_bundle_manifest_yaml` promoted to `pub(crate)`.
  - `BundleManifestFields` and its fields promoted to `pub(crate)`.
  - `admin_manifest_update_handler` now forwards `state.runtime.load()
    .config.trusted_signers` to `verify_integrity_payload_with_trusted`
    instead of passing an empty slice, allowing cluster-signed manifests to be
    accepted after the bundle-bootstrap flow has run.
- `tests/integrity_admin.rs`
  - 4 new unit tests for `parse_bundle_manifest_yaml` (minimal, missing block,
    ignored fields, dependency parsing).
  - 5 new async integration tests for `admin_manifest_bundle_handler`:
    empty body → 400, missing `manifest.yaml` → 400, valid bundle → 200,
    rollback → 409, dependency conflict → 428.
  - 3 existing `admin_manifest_update_*` tests updated to include the test
    signing key in `trusted_signers` of the seed config, matching the runtime
    trust model.
