## ADDED Requirements

### Requirement: Core host unwrap enforcement
The core host MUST reject new `unwrap()` calls in production and test code through crate-level linting and CI.

#### Scenario: Developer runs clippy locally
- **GIVEN** a developer introduces a new `unwrap()` in `core-host`
- **WHEN** `cargo clippy` runs for the workspace
- **THEN** the lint fails before the change can be merged

### Requirement: Centralized Tachyon error type
The core host MUST expose a centralized `TachyonError` type for incremental conversion from ad-hoc panic-prone paths to typed error propagation.

#### Scenario: A module needs to report a host error
- **GIVEN** a module encounters configuration, network, Wasm, header, I/O, or contextual failure
- **WHEN** it maps that failure into the host error model
- **THEN** it can return `TachyonResult<T>` without panicking
