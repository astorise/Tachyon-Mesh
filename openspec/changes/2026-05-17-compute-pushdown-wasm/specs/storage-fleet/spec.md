# storage-fleet Specification Delta

## ADDED Requirements

### Requirement: Pushdown Filter WIT
The mesh SHALL define a minimal `tachyon:storage@1.1.0/pushdown-filter` WIT interface whose `evaluate` function returns whether a key-value pair should be kept during a storage-local scan.

#### Scenario: Filter evaluates a row
- **WHEN** the storage node invokes `pushdown-filter.evaluate`
- **THEN** the returned boolean determines whether the row is serialized to the caller

### Requirement: Constrained Pushdown Scanner
The core host SHALL provide a bounded pushdown scanner path that accepts scan options and an optional Wasm filter payload while constraining execution through Wasmtime fuel configuration.

#### Scenario: Filter prunes rows locally
- **WHEN** scan options include a pushdown filter
- **THEN** rows rejected by the filter are skipped before network serialization
- **AND** the result still respects the requested scan limit

### Requirement: RedDB Direct Supports Filtered Scan
The `redb-direct` WIT interface SHALL expose `scan-filtered` with scan bounds, limit, and optional `pushdown-filter-wasm` bytes.

#### Scenario: Internal FaaS requests filtered scan
- **WHEN** an internal component calls `redb-direct.scan-filtered`
- **THEN** the host can apply the provided pushdown filter before returning rows

### Requirement: SQL Logical Plane Emits Pushdown Plan
The logical SQL FaaS proof of concept SHALL emit a bounded pushdown scan plan containing dummy filter bytes that can be consumed by the storage-local scanner.

#### Scenario: SQL engine compiles dummy suffix filter
- **WHEN** the SQL engine receives a request body
- **THEN** it returns a JSON scan plan containing `pushdown_filter_wasm`
