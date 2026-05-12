# Proposal: UX Polish and Cross-Layer CI Validation

## Why

Following the major security and functional remediation, two non-critical but important issues remain from the pre-release audit:
1. **Outdated Guided Tour:** `TachyonGuidedTour.ts` currently lacks guidance on the newly implemented atomic Seal & Apply pipeline, which is now the centerpiece of the configuration workflow.
2. **Missing Automated Consistency Checks:** The prior regressions involving phantom endpoints and unused WIT bindings demonstrate a need for automated cross-layer validation. We need a CI mechanism to ensure that requested client endpoints actually exist in the Rust `core-host` router.

## What Changes

### 1. UI UX Enhancements (Guided Tour & State)
- Update `TachyonGuidedTour.ts` to include a new step detailing the "Seal & Apply" atomic flow. The tour should highlight the visual diff modal and explain the Step-Up MFA process.
- Persist `lastMfaTimestamp` in `connectionStore` across reloads to prevent immediate re-authentication after a simple page refresh (cosmetic polish).
- Double-check `innerHTML` assignments in dynamic panels for proper XSS escaping.

### 2. CI Pipeline (Cross-Layer Validation)
- Add a new validation script to the CI pipeline (`scripts/validate_cross_layer.sh`).
- The script will statically analyze the `tachyon-client` `ADMIN_*_PATH` constants and ensure corresponding `.route(...)` handlers exist in `core-host/src/host_core/app_runtime.rs`.
- It will also verify that `tauri-plugin-stronghold` dependencies are accompanied by actual `Stronghold::` function calls to prevent future security theater regressions.
