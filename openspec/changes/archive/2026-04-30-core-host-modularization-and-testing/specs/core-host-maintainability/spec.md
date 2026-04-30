## ADDED Requirements

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

### Requirement: Coverage reporting in CI
The repository CI MUST expose a coverage generation step for core host changes.

#### Scenario: CI runs on a pull request
- **GIVEN** a pull request modifies Rust code
- **WHEN** the CI workflow reaches the coverage step
- **THEN** it installs `cargo-llvm-cov`
- **AND** emits an LCOV report artifact suitable for Codecov or equivalent uploaders
