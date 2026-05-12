# Tasks

## 1. UX Enhancements
- [x] Add a new tour step in `tachyon-ui/src/components/layout/TachyonGuidedTour.ts` for the "Seal & Apply" pipeline.
- [x] Update `tachyon-ui/src/stores/connectionStore.ts` to persist `lastMfaTimestamp` using local storage (or equivalent session storage) so it survives page reloads.
- [x] Audit `TachyonTopologyPanel.ts` and `TachyonUsersPanel.ts` templates to ensure `escapeHtml` is used consistently alongside any `innerHTML` assignments to prevent XSS.

## 2. CI Cross-Layer Validation
- [x] Create `scripts/validate_cross_layer.sh` to grep/awk `ADMIN_*_PATH` usage in `tachyon-client` and map them against `core-host/src/host_core/app_runtime.rs`.
- [x] Add a check in the same script for `Stronghold::` API usage in Tauri crates if the stronghold plugin is declared in dependencies.
- [x] Add a new step in `.github/workflows/ci.yml` to execute `bash scripts/validate_cross_layer.sh` on every pull request and push.