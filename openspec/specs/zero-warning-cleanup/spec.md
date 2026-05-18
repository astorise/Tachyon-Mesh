# zero-warning-cleanup Specification

## Purpose
TBD - created by archiving change v1-1-audit-absolute-polish-v2. Update Purpose after archive.
## Requirements
### Requirement: Remaining Component Lifecycle Tests
`core-host/tests/` SHALL contain `view_builder_test.rs`, `sql_engine_test.rs`, `vector_search_test.rs`, and `media_server_test.rs`, each performing a minimal Wasmtime boundary instantiation test for their respective component.

#### Scenario: All four scaffolded tests compile and run
- **GIVEN** the four new test files exist under `core-host/tests/`
- **WHEN** `cargo test --tests -p core-host` is invoked
- **THEN** each of the four tests SHALL load its target component successfully without panicking

### Requirement: No Unused biscuit-auth Dependency
`core-host/Cargo.toml` SHALL NOT include `biscuit-auth` as a default dependency. The crate SHALL either be removed or moved behind the `experimental` feature gate.

#### Scenario: Default build omits biscuit-auth
- **GIVEN** the repository at the audited state
- **WHEN** `cargo tree -p core-host --no-default-features` is inspected
- **THEN** `biscuit-auth` SHALL NOT appear in the dependency tree

### Requirement: Constrained Decoding Module Removal
The broken constrained-decoding FSM SHALL be removed from `core-host/src/ai_inference/samplers.rs`, and the matching WIT contracts SHALL be removed from the `wit/` tree.

#### Scenario: Sampler module has no broken FSM
- **GIVEN** the post-cleanup `samplers.rs`
- **WHEN** the file is inspected
- **THEN** the file SHALL NOT contain the previously flagged dummy FSM logic
- **AND** any retained code SHALL compile with `-D dead_code`

#### Scenario: WIT tree drops constrained-decoding contract
- **GIVEN** the `wit/` directory
- **WHEN** searching for `constrained-decoding` or equivalent stale interface
- **THEN** no such WIT contract SHALL remain

