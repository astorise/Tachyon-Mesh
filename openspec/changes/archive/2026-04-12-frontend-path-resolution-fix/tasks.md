# Tasks: Frontend Path Resolution Fix

- [x] Normalize the change artifacts to the current Tauri v2 vocabulary (`frontendDist`) and add a proper OpenSpec delta under `specs/`.
- [x] Verify that `tachyon-ui/tauri.conf.json` resolves frontend assets from the crate-local `dist` directory.
- [x] Validate the UI pipeline with `npm run build`, `npm run tauri build`, `cargo build`, and `openspec validate --all`.
