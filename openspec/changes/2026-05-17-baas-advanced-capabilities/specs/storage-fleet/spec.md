# storage-fleet Specification Delta

## ADDED Requirements

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
