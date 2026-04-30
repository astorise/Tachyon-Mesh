# core-host-maintainability Specification

## Purpose
TBD - created by archiving change core-host-modularization-and-testing. Update Purpose after archive.
## Requirements
### Requirement: Core host modular boundaries
The core host MUST expose a documented module layout that separates runtime, network, identity, storage, mesh, state, and telemetry concerns while preserving the current external host behavior during incremental refactors.

#### Scenario: New module directories exist
- **GIVEN** a contributor opens `core-host/src/`
- **WHEN** they inspect the source tree
- **THEN** runtime, network, identity, storage, mesh, and state modules are present
- **AND** existing telemetry and authentication modules remain directly consumable by the host entry point

### Requirement: Test infrastructure for routing behavior
The core host MUST provide reusable test dependencies and targeted tests for L4/L7 routing behavior.

#### Scenario: Routing behavior is validated locally
- **GIVEN** a contributor runs the core-host test suite
- **WHEN** L4 bind-address and L7 route-normalization tests execute
- **THEN** the tests validate stable routing behavior without requiring external network services

### Requirement: Core host monolith extraction progress
The core host MUST move concrete runtime, network, identity, telemetry, and state logic out of `main.rs` into dedicated modules while preserving existing host behavior.

#### Scenario: Extracted modules are compiled
- **WHEN** contributors build the `core-host` package
- **THEN** `state`, `identity`, `runtime`, `network`, and `telemetry` modules contain real host logic instead of placeholders
- **AND** the binary compiles with the entry point importing those module APIs

### Requirement: Production panic removal
The core host MUST avoid direct `panic!` macros in production source paths.

#### Scenario: Panic audit is run
- **WHEN** contributors search `core-host/src` for `panic!`
- **THEN** no production logic contains direct `panic!` macro calls
- **AND** fallible initialization paths return errors with context instead

### Requirement: Coverage reporting in CI
The repository CI MUST expose a coverage generation step for core host changes.

#### Scenario: CI runs on a pull request
- **GIVEN** a pull request modifies Rust code
- **WHEN** the CI workflow reaches the coverage step
- **THEN** it installs `cargo-llvm-cov`
- **AND** emits an LCOV report artifact suitable for Codecov or equivalent uploaders
