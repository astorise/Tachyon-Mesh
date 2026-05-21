## MODIFIED Requirements

### Requirement: CI runner installs the Node.js runtime, Rust toolchain, WASI target, and cache
The CI workflow SHALL install a pinned Node.js runtime for Tauri tooling, install the stable Rust toolchain with `rustfmt` and `clippy` components, add the `wasm32-wasip1` and `wasm32-wasip2` compilation targets, install Linux system dependencies including `protobuf-compiler`, and enable Rust dependency caching before building workspace artifacts.

#### Scenario: Runner is prepared for host, guest, and FIPS-adjacent compilation
- **WHEN** the CI workflow starts on a fresh runner
- **THEN** the pinned Node.js runtime is available to subsequent steps
- **AND** the stable Rust toolchain with `rustfmt` and `clippy` is available
- **AND** both `wasm32-wasip1` and `wasm32-wasip2` targets are installed
- **AND** Linux system dependencies including `protobuf-compiler` are installed
- **AND** Rust dependency caching is enabled to reduce repeated build time

## ADDED Requirements

### Requirement: CI runs a dedicated feature-matrix test job across multiple feature flag combinations
The CI workflow SHALL run a `feature-matrix-tests` job that tests `core-host` across at least five distinct feature flag combinations including default, `--no-default-features`, `--all-features`, `--features http3`, and a security bundle, uploading a release binary artifact for each combination.

#### Scenario: All feature combinations build and test successfully
- **WHEN** the feature-matrix-tests job runs
- **THEN** each matrix entry runs `cargo test -p core-host <features>` and `cargo build -p core-host --release <features>`
- **AND** each produces an uploaded artifact named `core-host-linux-x86_64-<label>-<sha>`

#### Scenario: All-features combination installs FIPS build dependencies
- **WHEN** the matrix entry with `--all-features` runs
- **THEN** it installs `cmake nasm protobuf-compiler` via apt before building

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
