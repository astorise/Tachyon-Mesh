## ADDED Requirements

### Requirement: Embedded core store supports vector indexing and ANN search
The `embedded-core-store` capability SHALL support indexing of high-dimensional vectors via a native Rust HNSW (Hierarchical Navigable Small World) implementation and expose Approximate Nearest Neighbor search over both in-memory and on-disk indices.

#### Scenario: Vector index ingests embeddings and answers ANN queries
- **WHEN** a tenant ingests vector embeddings into a configured vector index
- **THEN** the store builds and persists an HNSW index for that tenant
- **WHEN** an ANN search request is issued with a query vector and `k`
- **THEN** the store returns the top-`k` nearest neighbours along with their similarity scores
- **AND** the query latency is sub-millisecond for warm in-memory indices on typical Edge hardware

### Requirement: Vector access is exposed to Wasm guests via opt-in WIT interface
The Mesh SHALL expose a `wit/store/vector.wit` interface in the Wasm Component Model so that FaaS modules can opt into vector index access; modules that do not import the interface SHALL incur no runtime overhead from the vector subsystem.

#### Scenario: Module without vector import has no overhead
- **WHEN** a Wasm module is instantiated that does not import `wit/store/vector.wit`
- **THEN** the host does not allocate vector index resources for that module
- **AND** the module's invocation latency and memory footprint match the baseline FaaS profile

#### Scenario: Module with vector import performs an isolated similarity search
- **WHEN** a Wasm module imports `wit/store/vector.wit` and calls the search function
- **THEN** the host routes the call to the tenant's HNSW index
- **AND** returns the matching IDs and scores to the guest
- **AND** the index data remains isolated from other tenants according to the optional TDE policy
