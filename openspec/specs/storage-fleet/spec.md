# storage-fleet Specification

## Purpose
Expose storage, fleet, and air-gapped supply-chain configuration dashboards in the Tachyon web component shell.
## Requirements
### Requirement: Web component shell exposes Storage configuration
The Tachyon web component shell SHALL provide a `<tachyon-storage-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit WASI volume mount path and S3 proxy endpoint settings through the shared configuration command.

#### Scenario: Operator submits storage configuration
- **WHEN** the operator enters a WASI volume mount path and S3 proxy endpoint
- **THEN** the panel invokes `apply_configuration` with the storage domain
- **AND** the payload includes the mount path and endpoint values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Storage panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Storage route
- **AND** selecting that route mounts `<tachyon-storage-panel>`

### Requirement: Web component shell exposes Fleet configuration
The Tachyon web component shell SHALL provide a `<tachyon-fleet-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit node selector tags and a node profile through the shared configuration command.

#### Scenario: Operator submits fleet selector configuration
- **WHEN** the operator enters node selector tags and selects a node profile
- **THEN** the panel invokes `apply_configuration` with the fleet domain
- **AND** the payload includes selector tags and profile values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Fleet panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Fleet route
- **AND** selecting that route mounts `<tachyon-fleet-panel>`

### Requirement: Web component shell exposes Supply Chain configuration
The Tachyon web component shell SHALL provide a `<tachyon-supply-chain-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit asset signature key and air-gapped mode settings through the shared configuration command.

#### Scenario: Operator submits supply chain configuration
- **WHEN** the operator enters an asset signature key and toggles air-gapped mode
- **THEN** the panel invokes `apply_configuration` with the supply chain domain
- **AND** the payload includes the signature key and air-gapped mode flag
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Supply chain panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Supply Chain route
- **AND** selecting that route mounts `<tachyon-supply-chain-panel>`

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

### Requirement: CDC Broadcaster Enforces Subscriber Authorization
The mesh SHALL provide a CDC broadcaster component and `tachyon:storage@1.1.0/data-events` WIT contract for authorized subscribers to receive mutation events.

#### Scenario: Subscriber lacks a Biscuit bearer token
- **WHEN** a CDC mutation event is submitted without an authorization token
- **THEN** the broadcaster rejects the request
- **AND** authorized requests can forward mutation event payloads

### Requirement: Ephemeral Vector Search Component
The mesh SHALL provide a `system-faas-vector-search` component that computes cosine similarity over binary embedding rows inside an ephemeral Wasm instance.

#### Scenario: Vector query is executed
- **WHEN** the vector search component receives a query embedding and candidate embeddings
- **THEN** it returns the top scoring matches ordered by similarity

### Requirement: CRDT Conflict Resolution Hook
The storage layer SHALL expose a `tachyon:storage@1.1.0/conflict-resolution` WIT contract and detect vector-clock split-brain states before finalizing conflicting writes.

#### Scenario: Vector clocks diverge
- **WHEN** local and remote values for the same key have non-zero divergent vector clocks
- **THEN** the host invokes a conflict resolver
- **AND** commits the merged payload returned by the resolver

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

### Requirement: Subspace Access Tracker
The core host SHALL track local and remote access counts by key subspace and peer so hot remote access patterns can trigger migration planning without storing every individual key.

#### Scenario: Remote peer dominates a subspace
- **WHEN** remote hits for a subspace exceed the configured minimum and ratio over local hits
- **THEN** the tracker emits a migration plan targeting that peer

### Requirement: Zero-Downtime Migration Planning
The mesh migration layer SHALL represent a migration as a subspace and target peer plan that can be executed by the QUIC replication path without blocking normal reads.

#### Scenario: Migration plan is produced
- **WHEN** the access tracker crosses the migration threshold
- **THEN** the plan identifies the subspace and new target peer

### Requirement: Gossip Route Update Table
The mesh SHALL maintain a subspace routing table that accepts route updates and resolves the current primary peer for a key by longest matching subspace prefix.

#### Scenario: More specific subspace exists
- **WHEN** both `tenant` and `tenant/users` have primary peers
- **THEN** a key under `tenant/users` resolves to the more specific primary peer

### Requirement: Geo-Pinning Cooldown
The migration tracker SHALL apply a cooldown after emitting a migration plan so the same subspace cannot flap repeatedly between peers.

#### Scenario: Cooldown is active
- **WHEN** a subspace has just emitted a migration plan
- **THEN** repeated accesses during the cooldown do not emit another plan
