# storage-fleet Specification Delta

## ADDED Requirements

### Requirement: BaaS Query Authorization Cache
The core host SHALL integrate Biscuit authorization for BaaS query paths and cache sanitized query plus Datalog mutation results to avoid repeated SmolVM round trips for recurring requests.

#### Scenario: Repeated authorized query uses cache
- **WHEN** the same query and Datalog rule set is evaluated twice
- **THEN** the first request may invoke the AST mutation boundary
- **AND** the second request reuses the cached sanitized query result

### Requirement: Direct RedDB WIT Binding
The mesh SHALL expose a `tachyon:storage@1.1.0/redb-direct` WIT interface for trusted internal Wasm components to perform key-value get, put, and scan operations without the external gateway parser.

#### Scenario: Internal component scans RedDB directly
- **WHEN** a trusted internal component calls `redb-direct.scan`
- **THEN** the host returns bounded key-value rows from the requested table
- **AND** the external AST gateway is bypassed

### Requirement: RustFS Smart Compression Bypass
The storage layer SHALL detect common pre-compressed media and archive magic bytes and skip redundant compression for those blobs.

#### Scenario: PNG media is ingested
- **WHEN** a blob starts with the PNG magic bytes
- **THEN** RustFS stores the payload without applying zstd compression

### Requirement: Write-On-Read Schema Shim
The storage layer SHALL invoke a registered `schema-shim` WIT transformation when a fetched record carries a schema version older than the current version.

#### Scenario: Stale record is read
- **WHEN** a record with `schema_version` below the current schema version is retrieved
- **THEN** the host invokes the schema shim before returning the payload
- **AND** records at the current version are returned unchanged
