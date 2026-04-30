# ci-security Specification

## Purpose
TBD - created by archiving change ci-security-and-feature-matrix. Update Purpose after archive.
## Requirements
### Requirement: CI supply-chain checks
The CI workflow MUST run dependency vulnerability and policy checks before release artifacts are trusted.

#### Scenario: CI runs on main or pull request
- **GIVEN** a push or pull request triggers CI
- **WHEN** the security audit job runs
- **THEN** it executes `cargo audit`
- **AND** it executes `cargo deny` using the repository `deny.toml`

### Requirement: Feature matrix validation
The CI workflow MUST test the core host across default, no-default, all-feature, and selected feature combinations.

#### Scenario: Feature gated code changes
- **GIVEN** a change touches feature-gated host code
- **WHEN** CI executes the feature matrix job
- **THEN** `core-host` tests run for each configured feature set

### Requirement: Release SBOM
Release publishing MUST produce a Rust dependency SBOM artifact.

#### Scenario: Desktop release workflow runs
- **GIVEN** the release workflow builds desktop artifacts
- **WHEN** the Linux release job runs
- **THEN** it generates an SPDX JSON SBOM with `cargo-sbom`
- **AND** uploads it as a workflow artifact

### Requirement: Scheduled deep validation
The repository MUST schedule mutation and Miri validation for expensive safety checks.

#### Scenario: Weekly CI schedule fires
- **GIVEN** the scheduled workflow event runs
- **WHEN** deep validation jobs start
- **THEN** mutation tests target `core-host/src/auth.rs`
- **AND** Miri targets the cwasm cache deserialize smoke test

