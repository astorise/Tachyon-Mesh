## 1. Workflow Scaffold

- [x] 1.1 Create `.github/workflows/ci.yml` with triggers for pushes to `main` and pull requests targeting `main`.
- [x] 1.2 Configure the workflow to run on `ubuntu-latest`.
- [x] 1.3 Install the stable Rust toolchain and the `wasm32-wasip1` target before any build steps run.
- [x] 1.4 Add `Swatinem/rust-cache@v2` so repeated CI runs reuse compiled dependencies.

## 2. Quality Gates

- [x] 2.1 Run `cargo fmt --all -- --check` as a required formatting gate.
- [x] 2.2 Run `cargo clippy --workspace --all-targets --all-features -- -D warnings` so warnings fail the workflow.
- [x] 2.3 Run `cargo test --workspace` to validate the Rust workspace behavior.

## 3. Build Verification

- [x] 3.1 Build `guest-example` for the `wasm32-wasip1` target in release mode.
- [x] 3.2 Build `core-host` in release mode so the CI pipeline exercises the `build.rs` integrity embedding path.
- [x] 3.3 Confirm the workflow completes successfully on a clean checkout without requiring manual runner setup.
