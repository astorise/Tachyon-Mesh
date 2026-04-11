# github-actions Specification

## Purpose
TBD - created by archiving change github-actions. Update Purpose after archive.
## Requirements
### Requirement: Repository provides a GitHub Actions CI workflow for the main branch
The repository SHALL define a GitHub Actions workflow at `.github/workflows/ci.yml` that runs on pushes to `main` and pull requests targeting `main`.

#### Scenario: CI runs automatically for mainline changes
- **WHEN** a contributor pushes to `main` or opens or updates a pull request against `main`
- **THEN** GitHub Actions schedules the CI workflow automatically
- **AND** the workflow runs on a GitHub-hosted Linux runner

### Requirement: CI runner installs the Node.js runtime, Rust toolchain, WASI target, and cache
The CI workflow SHALL install a pinned Node.js runtime for Tauri tooling, install the stable Rust toolchain, add the `wasm32-wasip1` compilation target, and enable Rust dependency caching before building workspace artifacts.

#### Scenario: Runner is prepared for host and guest compilation
- **WHEN** the CI workflow starts on a fresh runner
- **THEN** the pinned Node.js runtime is available to subsequent steps
- **AND** the stable Rust toolchain is available to subsequent steps
- **AND** the `wasm32-wasip1` target is installed before the guest build runs
- **AND** Rust dependency caching is enabled to reduce repeated build time

### Requirement: CI enforces formatting, linting, tests, and production-oriented builds
The CI workflow SHALL fail when formatting, linting, workspace tests, the `guest-example` WASI build, the `core-host` release build, or the `tachyon-cli` release build do not succeed.

#### Scenario: CI validates the full Rust pipeline
- **WHEN** the workflow executes against a repository revision
- **THEN** it runs `cargo fmt --all -- --check`
- **AND** it runs `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- **AND** it runs `cargo test --workspace`
- **AND** it builds `guest-example` for `wasm32-wasip1` in release mode
- **AND** it builds `core-host` in release mode
- **AND** it builds `tachyon-cli` in release mode

### Requirement: CI publishes downloadable build artifacts
The CI workflow SHALL upload the primary release-oriented outputs so contributors can download the results of a successful build from GitHub Actions.

#### Scenario: CI persists build outputs after a successful run
- **WHEN** the workflow completes successfully
- **THEN** it uploads the sealed `integrity.lock` manifest as an artifact
- **AND** it uploads the release `core-host` binary as an artifact
- **AND** it uploads the release `tachyon-cli` binary as an artifact
- **AND** it uploads the release `guest-example` WASM module as an artifact

### Requirement: The repository builds Tachyon desktop bundles on every push and publishes release bundles on version tags
The repository SHALL define a GitHub Actions release workflow at `.github/workflows/release.yml` that builds the Tauri desktop application on Linux, macOS, and Windows runners for every push, uploads the resulting bundles as workflow artifacts on branch pushes, and publishes the resulting bundles to a draft GitHub Release when a semantic-version tag matching `v*` is pushed.

#### Scenario: A branch push triggers downloadable desktop workflow artifacts
- **WHEN** a contributor pushes a commit to any branch
- **THEN** GitHub Actions starts the desktop workflow automatically
- **AND** the workflow fans out across `ubuntu-22.04`, `macos-latest`, and `windows-latest`
- **AND** the generated Tauri bundles are uploaded as GitHub Actions workflow artifacts

#### Scenario: A release tag triggers a draft desktop release
- **WHEN** a maintainer pushes a tag such as `v1.0.0`
- **THEN** GitHub Actions starts the release workflow automatically
- **AND** the workflow fans out across `ubuntu-22.04`, `macos-latest`, and `windows-latest`
- **AND** the job has `contents: write` permission so it can create or update a GitHub Release draft

### Requirement: The release workflow provisions platform-specific build prerequisites for Tauri
The release workflow SHALL provision Node.js, the Rust toolchain, and platform-specific system dependencies required to bundle the Tauri desktop application from the `tachyon-cli` project directory.

#### Scenario: A Linux runner prepares the Tauri toolchain
- **WHEN** the release workflow executes on `ubuntu-22.04`
- **THEN** it installs Node.js 20
- **AND** it installs the stable Rust toolchain
- **AND** it installs `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libappindicator3-dev` or a compatible appindicator replacement, `librsvg2-dev`, and `patchelf`
- **AND** it installs frontend dependencies from the `tachyon-cli` directory

#### Scenario: A macOS runner prepares the Apple Silicon target
- **WHEN** the release workflow executes on `macos-latest`
- **THEN** it installs the stable Rust toolchain
- **AND** it adds the `aarch64-apple-darwin` Rust target before building the Tauri bundle

### Requirement: The Tauri project is configured to emit desktop bundles and updater artifacts
The `tachyon-cli` Tauri configuration SHALL enable bundling so the release workflow can build desktop installers on every supported operating system.

#### Scenario: Bundling is enabled for the desktop release pipeline
- **WHEN** the Tauri configuration is loaded from `tachyon-cli/tauri.conf.json`
- **THEN** `bundle.active` is enabled
- **AND** the configuration declares desktop bundle targets
- **AND** updater artifacts are enabled using the supported Tauri v2 configuration for updater bundles

### Requirement: The release workflow uses the official Tauri GitHub Action against the monorepo subproject
The release workflow SHALL use `tauri-apps/tauri-action@v0` with `projectPath: tachyon-cli`, pass `GITHUB_TOKEN`, upload workflow artifacts on ordinary pushes, and create a draft release populated with the platform bundles generated for each runner on semantic-version tags.

#### Scenario: Tauri artifacts are uploaded from the subproject path
- **WHEN** the release job invokes the Tauri GitHub Action
- **THEN** it uses the `tachyon-cli` project path instead of the repository root
- **AND** it receives `GITHUB_TOKEN` from `${{ secrets.GITHUB_TOKEN }}`
- **AND** it uploads workflow artifacts on branch pushes
- **AND** it uploads the generated release artifacts to a GitHub Release draft on version tags
