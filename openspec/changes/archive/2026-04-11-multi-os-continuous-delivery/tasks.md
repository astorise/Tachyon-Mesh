# Tasks: Change 060 Implementation

**Agent Instruction:** Implement the GitHub Actions release workflow for the Tauri application. You must ensure the paths match our monorepo structure.

## [TASK-1] Verify Tauri Configuration
- [x] Open `tachyon-cli/tauri.conf.json`.
- [x] Locate the `bundle` object.
- [x] Ensure `active` is set to `true`.
- [x] Ensure desktop bundle targets are enabled, using the Tauri v2-compatible updater setting instead of an invalid `updater` bundle target.

## [TASK-2] Create the Release Workflow File
- [x] Create a new file: `.github/workflows/release.yml`.
- [x] Name the workflow "Publish Tachyon Desktop".
- [x] Add the `on.push.tags` trigger for `v*`.
- [x] Grant `contents: write` permissions to the job (required to create the GitHub release).

## [TASK-3] Implement the Job Steps
Within the build matrix job, implement the following steps in order:
- [x] Add `actions/checkout@v4`.
- [x] Add `actions/setup-node@v4` configured to Node 20.
- [x] Add `dtolnay/rust-toolchain@stable` and configure the `aarch64-apple-darwin` target for macOS builds.
- [x] Install Linux dependencies via `sudo apt-get` only when `matrix.platform == 'ubuntu-22.04'`.
- [x] Run `npm install` explicitly in `working-directory: ./tachyon-cli`.
- [x] Invoke `tauri-apps/tauri-action@v0` with `projectPath: "tachyon-cli"` and draft release settings.

## Validation Step
- [x] Ensure the `working-directory` and `projectPath` correctly point to the `tachyon-cli` folder, not the root of the workspace.
- [x] Verify that the GitHub token environment variable is passed to the Tauri action (`GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}`).
- [x] Add a minimal Node package manifest for `tachyon-cli` so the workflow's `npm install` step is valid in this monorepo.
- [x] Validate the OpenSpec artifacts and the Rust build locally after the workflow changes.
