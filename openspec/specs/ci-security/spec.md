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

#### Scenario: Temporarily ignored advisories are documented
- **GIVEN** an upstream dependency pins a vulnerable transitive crate below the fixed version
- **WHEN** the security audit job ignores the advisory temporarily
- **THEN** the CI workflow documents the affected advisory IDs and blocking parent crates
- **AND** `deny.toml` records matching ignore entries with the condition for removing them

#### Scenario: Fixed transitive advisories are no longer ignored
- **GIVEN** upstream parent crates can resolve the transitive dependency to the fixed version
- **WHEN** the lockfile is updated to the fixed dependency line
- **THEN** the security audit job MUST run without ignores for those fixed advisory IDs
- **AND** `deny.toml` MUST NOT keep stale ignore entries for those advisory IDs
- **AND** `cargo deny` MUST allow any temporary patched sources required to reach the fixed dependency line

#### Scenario: Direct TLS parser advisory is migrated to maintained APIs
- **GIVEN** a RustSec advisory marks a direct TLS PEM parsing crate as unmaintained
- **WHEN** the workspace already depends on maintained PEM parsing APIs through the rustls PKI types
- **THEN** host TLS code SHALL migrate direct PEM parsing to `rustls::pki_types::pem::PemObject`
- **AND** direct dependencies on the unmaintained parser crate SHALL be removed from package manifests and `Cargo.lock`
- **AND** `deny.toml` SHALL remove any ignore entry for the remediated advisory

#### Scenario: Unmaintained dependencies remain only through latest upstream parents
- **GIVEN** `cargo audit` reports an unmaintained crate reachable only through a third-party parent crate
- **WHEN** the parent crate is already at its latest published compatible version
- **THEN** the repository SHALL leave the transitive dependency to upstream rather than patching unrelated crates locally
- **AND** the dependency tree SHALL identify the blocking parent crate for follow-up

#### Scenario: Coordinated RustCrypto migrations are isolated from weekly dependency groups
- **GIVEN** RustCrypto crates share public `rand_core`, `signature`, DER, SPKI, PKCS#8, or digest trait families across Tachyon packages
- **WHEN** Renovate proposes a next-generation RustCrypto wave that changes those trait families
- **THEN** the affected crates SHALL be migrated and validated together in a dedicated change
- **AND** direct randomness usage SHALL use the published OS RNG API for the selected `rand`/`rand_core` generation
- **AND** crates blocked by an upstream trait-family mismatch, such as `rsa` remaining on an older `digest` family, SHALL stay pinned with an explanatory manifest comment until the blocking parent crate supports the newer family

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

### Requirement: CI validates cross-layer admin contracts
The CI workflow SHALL run a repository script that verifies Tachyon client admin endpoint constants are backed by matching core-host admin routes before build-heavy jobs proceed.

#### Scenario: Client admin endpoint has a host route
- **GIVEN** `tachyon-client/src/lib.rs` defines an `ADMIN_*_PATH` constant
- **WHEN** CI runs cross-layer validation
- **THEN** `scripts/validate_cross_layer.sh` verifies that `core-host/src/host_core/admin_plane.rs` contains the exact route literal or a dynamic route beneath that path

#### Scenario: Client admin endpoint is missing from the host
- **GIVEN** `tachyon-client/src/lib.rs` defines an `ADMIN_*_PATH` constant with no matching host route
- **WHEN** CI runs cross-layer validation
- **THEN** the script exits non-zero
- **AND** prints the missing endpoint to standard error

### Requirement: CI verifies Stronghold is actively used when declared
The CI workflow SHALL fail when the Tauri UI declares `tauri-plugin-stronghold` without active `Stronghold::` API usage in the Tauri Rust entrypoint.

#### Scenario: Stronghold dependency has active usage
- **GIVEN** `tachyon-ui/Cargo.toml` declares `tauri-plugin-stronghold`
- **WHEN** CI runs cross-layer validation
- **THEN** the script verifies `tachyon-ui/src/main.rs` contains a `Stronghold::` invocation

#### Scenario: Stronghold dependency is unused
- **GIVEN** `tachyon-ui/Cargo.toml` declares `tauri-plugin-stronghold`
- **AND** `tachyon-ui/src/main.rs` does not contain a `Stronghold::` invocation
- **WHEN** CI runs cross-layer validation
- **THEN** the script exits non-zero

