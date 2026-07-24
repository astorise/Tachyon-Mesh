# Embedded Core Store

## Purpose
Define how Tachyon Mesh persists internal host control data in an embedded ACID store so runtime state survives restarts without external infrastructure.
## Requirements
### Requirement: The host persists internal runtime state in an embedded ACID key-value store
The host SHALL use an embedded persistent store for crash-resilient internal state such as compiled module cache entries, certificate material, and hibernation data.

#### Scenario: The host needs to persist internal control data
- **WHEN** the runtime stores compiled artifacts, certificate records, or suspended state
- **THEN** it writes them into the embedded core store with crash-safe persistence semantics
- **AND** can recover them after a restart

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

### Requirement: Repository provides an end-to-end RAG vector guest example
The repository SHALL provide an `examples/guest-rag-vector` user FaaS example that demonstrates document ingestion, embedding generation, vector upsert/search through `tachyon:mesh/vector`, and answer generation with retrieved context. The example SHALL try the OpenAI-compatible `/ai/v1/embeddings` and `/ai/v1/chat/completions` routes through scoped outbound HTTP when they are available, and SHALL provide deterministic local fallbacks so the route remains smoke-testable without a loaded model.

#### Scenario: RAG guest indexes documents and returns nearest context
- **WHEN** `/api/guest-rag-vector` receives a JSON request with `query`, `index`, `topK`, and optional `documents`
- **THEN** the guest creates or reuses the named vector index
- **AND** upserts document embeddings with payload text
- **AND** searches the index with the query embedding
- **AND** returns `matches` containing document IDs, scores, and payload text

#### Scenario: RAG guest uses OpenAI-compatible routes when available
- **GIVEN** the route's deployment scopes grant outbound HTTP access to `/ai/v1/embeddings` and `/ai/v1/chat/completions`
- **WHEN** those routes return successful OpenAI-compatible responses
- **THEN** `guest-rag-vector` uses their embeddings and completion content
- **AND** the response identifies the OpenAI-compatible embedding and completion sources

#### Scenario: RAG guest falls back without a loaded model
- **GIVEN** the OpenAI-compatible routes are absent, unavailable, or return unusable payloads
- **WHEN** `/api/guest-rag-vector` handles a request
- **THEN** it computes deterministic local embeddings
- **AND** returns a fallback answer grounded in the best retrieved match
