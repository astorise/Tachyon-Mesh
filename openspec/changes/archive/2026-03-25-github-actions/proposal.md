## Why

Tachyon Mesh depends on a narrow Rust toolchain, compile-time integrity embedding, and WASI guest builds. We need a reproducible GitHub-hosted validation pipeline so pull requests cannot regress formatting, linting, tests, or release-oriented build behavior.

## What Changes

- Add a new `github-actions` capability that defines the baseline CI workflow for the repository.
- Require a GitHub Actions workflow triggered by pushes and pull requests against `main`.
- Validate formatting, linting, workspace tests, the `guest-example` WASI build, and the `core-host` release build in one pipeline.
- Standardize the runner setup around the stable Rust toolchain, the `wasm32-wasip1` target, and Rust dependency caching.

## Capabilities

### New Capabilities

- `github-actions`: Continuous integration workflow for formatting, linting, testing, and production-oriented build verification.

### Modified Capabilities

- None.

## Impact

- Adds `.github/workflows/ci.yml` to the repository.
- Requires GitHub-hosted runners to install the `wasm32-wasip1` target before guest compilation.
- Exercises `core-host` release builds so the `build.rs` path that embeds `integrity.lock` stays healthy.
