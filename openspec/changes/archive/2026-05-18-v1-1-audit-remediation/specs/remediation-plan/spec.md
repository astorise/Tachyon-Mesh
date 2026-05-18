# remediation-plan Specification Delta

## ADDED Requirements

### Requirement: CDC Broadcaster Fail-Closed Authentication
The `system-faas-cdc-broadcaster` SHALL reject all unauthenticated requests with `401 Unauthorized` and SHALL NOT accept any non-empty bearer token as proof of identity.

#### Scenario: Arbitrary bearer token is rejected
- **GIVEN** the broadcaster receives an HTTP request with header `Authorization: Bearer placeholder`
- **WHEN** authentication is evaluated
- **THEN** the broadcaster SHALL return HTTP status `401`
- **AND** the broadcaster SHALL NOT broadcast the underlying payload

#### Scenario: Missing Authorization header is rejected
- **GIVEN** the broadcaster receives an HTTP request without an `Authorization` header
- **WHEN** authentication is evaluated
- **THEN** the broadcaster SHALL return HTTP status `401`

### Requirement: SubspaceAccessTracker Bounded Memory
The `SubspaceAccessTracker` in `core-host/src/mesh/migration.rs` SHALL bound its internal map to at most `10000` entries to prevent unbounded memory growth.

#### Scenario: Overflow trims oldest tracker entries
- **GIVEN** the access tracker already contains `10000` entries
- **WHEN** a new subspace access is recorded
- **THEN** the tracker SHALL evict or refuse entries so the size remains bounded by `10000`

### Requirement: OLAP Engine Payload Size Limit
The `system-faas-olap-engine` SHALL refuse JSON payloads larger than the configured byte ceiling (default `2 MiB`) before deserialization.

#### Scenario: Oversized JSON payload is rejected
- **GIVEN** a client submits a JSON payload exceeding `2 MiB`
- **WHEN** the olap engine reads the request body
- **THEN** the engine SHALL abort the request with a payload-too-large error
- **AND** the engine SHALL NOT call `serde_json::from_slice` on the data

### Requirement: Pinned Rust Toolchain
The repository SHALL pin a Rust toolchain version via a `rust-toolchain.toml` file at the workspace root.

#### Scenario: Build uses pinned toolchain
- **GIVEN** a developer runs `cargo build` at the workspace root
- **WHEN** `rustup` resolves the toolchain
- **THEN** rustup SHALL select the channel declared in `rust-toolchain.toml`
- **AND** the file SHALL include `rustfmt` and `clippy` components

### Requirement: Truthful Archive Audit Trail
Archived OpenSpec change task lists SHALL accurately reflect the implementation state, with no `[x]` marks on items whose code is unwired or stubbed.

#### Scenario: Ghost feature tasks are unchecked
- **GIVEN** an archived change whose tasks were marked complete but whose code is gated under `#[allow(dead_code)]`
- **WHEN** the audit trail is reconciled
- **THEN** every false-positive `[x]` task SHALL be reverted to `[ ]`

### Requirement: Experimental Feature Gate for Unwired Stubs
`core-host` SHALL expose an `experimental` Cargo feature, and every unwired stub previously hidden by `#[allow(dead_code)]` SHALL be gated by `#[cfg(feature = "experimental")]` instead.

#### Scenario: Experimental feature gates dead code
- **GIVEN** a module in `core-host/src` contains code formerly tagged `#[allow(dead_code)]`
- **WHEN** the workspace builds with default features
- **THEN** the unwired stub SHALL be excluded from compilation
- **AND** building with `--features experimental` SHALL compile the stub successfully
