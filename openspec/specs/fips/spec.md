# fips Specification

## Purpose
TBD - created by archiving change fips-musl-alpine. Update Purpose after archive.
## Requirements
### Requirement: core-host can be compiled with FIPS 140-3 compliant cryptography
The build system SHALL support compiling `core-host` with `--features fips`, which substitutes the default `ring`-based TLS backend with `aws-lc-fips-sys` providing FIPS 140-3 validated cryptographic primitives.

#### Scenario: FIPS feature compiles successfully
- **WHEN** a developer runs `cargo build -p core-host --features fips`
- **THEN** the build succeeds without errors
- **AND** the resulting binary uses `aws-lc-fips-sys` for all TLS operations

#### Scenario: FIPS and ring features are mutually exclusive
- **WHEN** a developer attempts to enable both `fips` and `ring`-dependent features simultaneously
- **THEN** the build fails with a clear cargo feature conflict error

### Requirement: A dedicated Dockerfile.fips produces a FIPS-compliant scratch image
The repository SHALL provide a `Dockerfile.fips` that uses `rust:alpine` as the FIPS builder stage to compile `core-host --features fips` with musl libc, producing a statically-linked binary in a `FROM scratch` final image under 35 MB.

#### Scenario: Docker FIPS image builds successfully
- **WHEN** `docker build -f Dockerfile.fips .` is executed
- **THEN** the build completes using Alpine's native musl toolchain
- **AND** the final image is based on `FROM scratch`
- **AND** the final image size is under 35 MB

#### Scenario: FIPS image contains only necessary files
- **WHEN** the FIPS Docker image is inspected
- **THEN** it contains the statically-linked `core-host` binary
- **AND** it contains the required WASM modules
- **AND** it contains no shell, package manager, or OS utilities

### Requirement: CI validates the FIPS build on every push
The CI workflow SHALL include a dedicated `fips-tests` job that installs FIPS build dependencies (cmake, nasm, protobuf-compiler), runs `cargo test -p core-host --features fips`, builds a release binary, and uploads it as a CI artifact.

#### Scenario: fips-tests job runs and uploads artifact
- **WHEN** a push or pull request triggers CI
- **THEN** the `fips-tests` job installs cmake, nasm, and protobuf-compiler
- **AND** runs `cargo test -p core-host --features fips`
- **AND** builds `cargo build -p core-host --release --features fips`
- **AND** uploads the resulting binary as `core-host-linux-x86_64-fips-<sha>`

### Requirement: FIPS Docker image is published to GHCR on main push
The CI workflow SHALL build and publish `ghcr.io/<owner>/tachyon-mesh:latest-fips` using `Dockerfile.fips` on every push to `main`.

#### Scenario: FIPS image published after successful CI
- **WHEN** a commit is pushed to `main` and `rust-ci` passes
- **THEN** the `publish-docker-images` job builds the FIPS variant using `Dockerfile.fips`
- **AND** pushes it tagged as `latest-fips` and `sha-<sha>-fips`

