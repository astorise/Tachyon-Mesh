## 1. Change Repair

- [x] 1.1 Rewrite the proposal, design, and tasks artifacts so `tauri-roles-update` matches the current role-aware `tachyon-cli` behavior.
- [x] 1.2 Replace the invalid flat `specs.md` artifact with a valid delta spec under `specs/tauri-configurator/spec.md`.

## 2. Verification

- [x] 2.1 Verify `tachyon-cli` continues to accept `--route` plus `--system-route` and emits sealed `user` and `system` roles in `integrity.lock`.
- [x] 2.2 Run `openspec validate --all`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo build -p core-host --release`, and `cargo build -p tachyon-cli --release`.

## 3. Delivery

- [x] 3.1 Commit and push the repaired change on the active PR branch.
- [x] 3.2 Wait for the GitHub Actions checks on the PR to return green, fixing any regression before archive.
- [x] 3.3 Archive `tauri-roles-update` with spec sync once the branch is green.
