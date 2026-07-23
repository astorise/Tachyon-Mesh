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
The CI workflow SHALL install a pinned Node.js runtime for Tauri tooling, install the stable Rust toolchain with `rustfmt` and `clippy` components, add the `wasm32-wasip1` and `wasm32-wasip2` compilation targets, install Linux system dependencies including `protobuf-compiler`, and enable Rust dependency caching before building workspace artifacts.

#### Scenario: Runner is prepared for host, guest, and FIPS-adjacent compilation
- **WHEN** the CI workflow starts on a fresh runner
- **THEN** the pinned Node.js runtime is available to subsequent steps
- **AND** the stable Rust toolchain with `rustfmt` and `clippy` is available
- **AND** both `wasm32-wasip1` and `wasm32-wasip2` targets are installed
- **AND** Linux system dependencies including `protobuf-compiler` are installed
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

### Requirement: The Tauri project is configured to emit desktop bundles
The `tachyon-cli` Tauri configuration SHALL enable bundling so the release workflow can build desktop installers on every supported operating system.

#### Scenario: Bundling is enabled for the desktop release pipeline
- **WHEN** the Tauri configuration is loaded from `tachyon-cli/tauri.conf.json`
- **THEN** `bundle.active` is enabled
- **AND** the configuration declares desktop bundle targets
- **AND** the configuration avoids updater-only bundle settings until the updater plugin is configured

### Requirement: The release workflow uses the official Tauri GitHub Action against the monorepo subproject
The release workflow SHALL use `tauri-apps/tauri-action@v0` with `projectPath: tachyon-cli`, pass `GITHUB_TOKEN`, upload workflow artifacts on ordinary pushes, and create a draft release populated with the platform bundles generated for each runner on semantic-version tags.

#### Scenario: Tauri artifacts are uploaded from the subproject path
- **WHEN** the release job invokes the Tauri GitHub Action
- **THEN** it uses the `tachyon-cli` project path instead of the repository root
- **AND** it receives `GITHUB_TOKEN` from `${{ secrets.GITHUB_TOKEN }}`
- **AND** it uploads workflow artifacts on branch pushes
- **AND** it uploads the generated release artifacts to a GitHub Release draft on version tags

### Requirement: CI validates the renamed desktop wrapper
The CI workflow SHALL validate the renamed `tachyon-ui` desktop wrapper and keep the workspace references aligned with the new client triad layout.

#### Scenario: CI builds the renamed desktop project
- **WHEN** the CI workflow runs on GitHub Actions
- **THEN** it builds `tachyon-ui` in release mode
- **AND** release bundling uses the `tachyon-ui` project path

### Requirement: Release workflow bundles the renamed desktop project
The desktop release workflow SHALL build the Tauri bundles from the `tachyon-ui` project directory on each supported operating system.

#### Scenario: The release workflow targets the renamed desktop directory
- **WHEN** the release workflow invokes the Tauri action
- **THEN** `projectPath` points to `tachyon-ui`
- **AND** frontend dependencies are installed from the `tachyon-ui` directory

### Requirement: Release workflow publishes versioned integrity schema assets
The release workflow SHALL publish `integrity-config.schema.json` and `integrity-lock.schema.json` as GitHub Release assets for version tags. The workflow SHALL generate those files from the checked-out source using `core-host schema` and stamp their `$id` values with the pushed release tag.

#### Scenario: Release tag uploads manifest schema assets
- **WHEN** a maintainer pushes a tag such as `v1.2.3`
- **THEN** the release workflow runs a schema publishing job on a Linux runner
- **AND** it generates `integrity-config.schema.json` and `integrity-lock.schema.json`
- **AND** it uploads both files to the GitHub Release for that tag

### Requirement: CI checks manifest schema compatibility against the latest release
The CI workflow SHALL generate HEAD integrity schemas and compare them with the latest non-draft, non-prerelease GitHub Release schema assets when both previous assets exist. Adding required fields, removing properties, changing schema types, or removing enum values SHALL fail CI unless the pull request is explicitly labeled `breaking-manifest`.

#### Scenario: Backward-incompatible manifest schema change fails CI
- **WHEN** a pull request changes the generated manifest schema incompatibly with the latest release assets
- **AND** the pull request does not carry the `breaking-manifest` label
- **THEN** the manifest schema compatibility job fails and reports the breaking paths

#### Scenario: Missing historical schema assets skip the diff
- **WHEN** the latest release does not include both integrity schema assets
- **THEN** CI records a notice and skips the compatibility diff rather than failing unrelated changes

### Requirement: Release workflow MUST publish Windows binaries as .zip
The `publish-server-binaries` matrix SHALL include `windows-latest / x86_64-pc-windows-msvc`. The packaging step SHALL produce `tachyon-mesh-VERSION-windows-x86_64.zip` containing `core-host.exe` and `tachyon-mcp.exe`.

#### Scenario: Windows release artifact is zipped and uploaded
- **WHEN** the release workflow runs on `windows-latest`
- **THEN** it compiles `core-host` and `tachyon-mcp` for `x86_64-pc-windows-msvc`
- **AND** packages them as `tachyon-mesh-<version>-windows-x86_64.zip`
- **AND** uploads the zip as a release artifact

### Requirement: CI runs a dedicated feature-matrix test job across multiple feature flag combinations
The CI workflow SHALL run a `feature-matrix-tests` job that tests `core-host` across at least six distinct feature flag combinations including default, `--no-default-features`, `--all-features`, `--features http3`, a security bundle, and a worker data-plane profile (`--no-default-features` plus the transport features a mesh member needs with `admin-plane` omitted), uploading a release binary artifact for each combination.

#### Scenario: All feature combinations build and test successfully
- **WHEN** the feature-matrix-tests job runs
- **THEN** each matrix entry runs `cargo test -p core-host <features>` and `cargo build -p core-host --release <features>`
- **AND** each produces an uploaded artifact named `core-host-linux-x86_64-<label>-<sha>`

#### Scenario: All-features combination installs FIPS build dependencies
- **WHEN** the matrix entry with `--all-features` runs
- **THEN** it installs `cmake nasm protobuf-compiler` via apt before building

#### Scenario: Worker profile combination has no admin surface
- **WHEN** the worker-profile matrix entry runs
- **THEN** it builds and tests `core-host` with `admin-plane` disabled
- **AND** the router-level test suite confirms `/admin/*` routes other than enrollment bootstrap are unreachable

### Requirement: CI includes a dedicated FIPS test job
The CI workflow SHALL include a `fips-tests` job that installs FIPS build dependencies and validates `core-host --features fips` independently of the main `rust-ci` job.

#### Scenario: fips-tests job executes independently
- **WHEN** CI is triggered by a push or pull request
- **THEN** the `fips-tests` job runs in parallel with `rust-ci` and `feature-matrix-tests`
- **AND** it installs cmake, nasm, and protobuf-compiler
- **AND** it runs `cargo test -p core-host --features fips` and uploads the release binary

### Requirement: Docker publish job includes a FIPS image variant
The `publish-docker-images` job SHALL include a matrix entry that builds `Dockerfile.fips` and publishes it tagged with the `-fips` suffix to GHCR on every push to `main`.

#### Scenario: FIPS Docker variant published alongside standard variants
- **WHEN** a commit is merged to `main` and `rust-ci` passes
- **THEN** the Docker publish matrix builds four variants: default, `-fips`, `-http3`, `-security`
- **AND** the `-fips` variant uses `Dockerfile.fips` as its Dockerfile
- **AND** all variants are pushed to `ghcr.io/<owner>/tachyon-mesh` with appropriate tags

### Requirement: CI validates feature-gated core-host builds in the quality gate
The CI workflow SHALL include compile-only `cargo check -p core-host --features <feature>` checks in the `quality` job before heavier lint, cross-layer validation, integration test, and downstream matrix jobs. The checks SHALL cover the CPU feature-gated paths exercised by the downstream feature matrix that are not covered by the workspace lint feature set: `http3`, `mtls`, `rate-limit`, `resiliency`, `s3-persistence`, `secrets-vault`, and `websockets`.

#### Scenario: feature compile checks run early in quality
- **WHEN** the CI workflow runs on GitHub Actions
- **THEN** the `quality` job runs `cargo check -p core-host --features <feature>` for each required feature before workspace linting
- **AND** the step fails the build on any compilation error before downstream feature-matrix jobs are scheduled

### Requirement: feature-matrix-tests includes s3-persistence combination
The `feature-matrix-tests` job SHALL include `--features s3-persistence` as one of its matrix entries, uploading the resulting binary as a labeled artifact.

#### Scenario: s3-persistence matrix entry builds and uploads artifact
- **WHEN** the feature-matrix-tests job runs the s3-persistence entry
- **THEN** it runs `cargo test -p core-host --features s3-persistence` and `cargo build -p core-host --release --features s3-persistence`
- **AND** uploads the binary as `core-host-linux-x86_64-s3-persistence-<sha>`
