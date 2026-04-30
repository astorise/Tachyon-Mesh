## ADDED Requirements

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
