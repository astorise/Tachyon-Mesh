# Implementation Tasks

## Phase 1: Dependency & Supply Chain Security
- [x] Add a `deny.toml` configuration file to ban unwanted licenses (e.g., GPL-3.0 in a commercial setting) and unmaintained crates.
- [x] Add `cargo-audit` and `cargo-deny` steps to `.github/workflows/ci.yml`.
- [x] Implement an SBOM generation step in `.github/workflows/release.yml` using `cargo-sbom`.

## Phase 2: Feature Matrix & Mutation Testing
- [x] Refactor the `cargo test` command in CI to use a GitHub Actions matrix testing all feature flag combinations.
- [x] Install `cargo-mutants` locally, run it against `auth.rs`, and write any missing tests to cover the surviving mutants. Add a weekly mutant run to CI.

## Phase 3: Miri (Undefined Behavior)
- [x] Ensure the `cwasm_cache` module has dedicated unit tests invoking the `unsafe { Component::deserialize }` logic.
- [x] Add the nightly Miri job to CI specifically targeting these tests to ensure no memory leaks or segfaults exist.
