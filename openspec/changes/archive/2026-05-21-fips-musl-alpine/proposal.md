## Why

Building `core-host --features fips` on glibc-based Ubuntu failed because `aws-lc-fips-sys` requires musl or specific header alignment that Alpine's native musl toolchain provides cleanly. A dedicated `Dockerfile.fips` using `rust:alpine` as the FIPS builder resolves this and produces a statically-linked, scratch-based final image (~32 MB) suitable for air-gapped, FIPS-compliant deployments.

## What Changes

- **New `Dockerfile.fips`**: multi-stage build with `rust:alpine` as `fips-builder` (installs cmake, nasm, go, perl, linux-headers) and Ubuntu as `wasm-builder`; final stage `FROM scratch` copies only the FIPS-compiled `core-host` binary and WASM modules.
- **CI `fips-tests` job**: dedicated GitHub Actions job that installs `cmake nasm protobuf-compiler` and runs `cargo test -p core-host --features fips` + release build, uploading the artifact.
- **CI `publish-docker-images` matrix**: adds a `-fips` variant entry that uses `Dockerfile.fips` and publishes `ghcr.io/<owner>/tachyon-mesh:latest-fips`.
- **`rust-ci` and `feature-matrix-tests`**: `protobuf-compiler` added to `apt-get` since `--all-features` now includes `prost`-dependent code.

## Capabilities

### New Capabilities

- `fips`: FIPS 140-3 compliant TLS via `aws-lc-fips-sys` + Alpine musl build pipeline producing a static scratch image.

### Modified Capabilities

- `github-actions`: CI matrix extended with `fips-tests` job and `-fips` Docker publish variant.

## Impact

- **Dockerfile.fips** (new file): Alpine musl builder → scratch runtime, ~32 MB image.
- **`.github/workflows/ci.yml`**: new `fips-tests` job; `protobuf-compiler` added to existing jobs; Docker matrix extended.
- **`core-host/Cargo.toml`**: `fips` feature definition unchanged; build now validated on musl via CI.
- No API or WIT interface changes.
