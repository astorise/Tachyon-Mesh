## ADDED Requirements

### Requirement: The repository publishes Tachyon desktop release bundles from a tag-driven multi-OS workflow
The repository SHALL define a GitHub Actions release workflow at `.github/workflows/release.yml` that only runs when a semantic-version tag matching `v*` is pushed, builds the Tauri desktop application on Linux, macOS, and Windows runners, and publishes the resulting bundles to a draft GitHub Release.

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
The release workflow SHALL use `tauri-apps/tauri-action@v0` with `projectPath: tachyon-cli`, pass `GITHUB_TOKEN`, and create a draft release populated with the platform bundles generated for each runner.

#### Scenario: Tauri artifacts are uploaded from the subproject path
- **WHEN** the release job invokes the Tauri GitHub Action
- **THEN** it uses the `tachyon-cli` project path instead of the repository root
- **AND** it receives `GITHUB_TOKEN` from `${{ secrets.GITHUB_TOKEN }}`
- **AND** it uploads the generated release artifacts to a GitHub Release draft
