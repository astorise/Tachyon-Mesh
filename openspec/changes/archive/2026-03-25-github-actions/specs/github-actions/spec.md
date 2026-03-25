## ADDED Requirements

### Requirement: Repository provides a GitHub Actions CI workflow for the main branch
The repository SHALL define a GitHub Actions workflow at `.github/workflows/ci.yml` that runs on pushes to `main` and pull requests targeting `main`.

#### Scenario: CI runs automatically for mainline changes
- **WHEN** a contributor pushes to `main` or opens or updates a pull request against `main`
- **THEN** GitHub Actions schedules the CI workflow automatically
- **AND** the workflow runs on a GitHub-hosted Linux runner

### Requirement: CI runner installs the Rust toolchain, WASI target, and cache
The CI workflow SHALL install the stable Rust toolchain, add the `wasm32-wasip1` compilation target, and enable Rust dependency caching before building workspace artifacts.

#### Scenario: Runner is prepared for host and guest compilation
- **WHEN** the CI workflow starts on a fresh runner
- **THEN** the stable Rust toolchain is available to subsequent steps
- **AND** the `wasm32-wasip1` target is installed before the guest build runs
- **AND** Rust dependency caching is enabled to reduce repeated build time

### Requirement: CI enforces formatting, linting, tests, and production-oriented builds
The CI workflow SHALL fail when formatting, linting, workspace tests, the `guest-example` WASI build, or the `core-host` release build do not succeed.

#### Scenario: CI validates the full Rust pipeline
- **WHEN** the workflow executes against a repository revision
- **THEN** it runs `cargo fmt --all -- --check`
- **AND** it runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- **AND** it runs `cargo test --workspace`
- **AND** it builds `guest-example` for `wasm32-wasip1` in release mode
- **AND** it builds `core-host` in release mode
