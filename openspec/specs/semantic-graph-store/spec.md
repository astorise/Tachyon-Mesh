# semantic-graph-store Specification

## Purpose
Provides a host-side hexastore (Subject-Predicate-Object graph) backed by redb and exposed to Wasm FaaS guests via the `graph::workspace-graph` WIT resource. Multi-hop BFS traversal executes entirely on the host to prevent large graph serialisation into guest memory.

## Requirements

### Requirement: WIT contract MUST define a workspace-graph resource
The `graph` interface SHALL declare an `edge` record (`subject`, `predicate`, `object`, `properties: string`) and a `workspace-graph` resource with constructor, `add-edges`, `delete-edges`, and `traverse` functions. The interface SHALL be imported into the `faas-guest` world.

#### Scenario: Guest creates a graph handle
- **WHEN** a FaaS guest calls `workspace-graph(name: "my-graph")`
- **THEN** the host allocates a `WorkspaceGraphResource` and returns a typed resource handle
- **AND** the handle is tracked in the Wasmtime `ResourceTable`

### Requirement: Graph edges MUST be stored in dual SPO and OSP redb indices
The host SHALL maintain two dynamic `redb::TableDefinition<&[u8], &str>` tables per namespace: `graph_{name}_spo` (key: `S\0P\0O`) and `graph_{name}_osp` (key: `O\0S\0P`). Null-byte separators enable efficient prefix scanning.

#### Scenario: Edges are indexed in both directions
- **GIVEN** an edge `(alice, knows, bob)` is added
- **WHEN** the SPO table is prefix-scanned for `alice\0knows\0`
- **THEN** `bob` is returned
- **WHEN** the OSP table is prefix-scanned for `bob\0alice\0`
- **THEN** the edge is found in the reverse index

### Requirement: Mutations MUST be atomic across both indices
`add-edges` and `delete-edges` SHALL operate within a single `redb::WriteTransaction`, inserting or removing records from SPO and OSP atomically before calling `.commit()`.

#### Scenario: Partial failure leaves no inconsistent state
- **GIVEN** a write transaction starts
- **WHEN** the transaction is aborted before commit
- **THEN** neither the SPO nor the OSP table is modified

### Requirement: Traversal MUST use BFS with a visited set and result cap
`traverse(subject, predicate, depth)` SHALL perform breadth-first search from `subject`, following `predicate` edges up to `depth` hops. A `HashSet<String>` prevents revisiting nodes. The result set is capped at 10,000 objects to prevent OOM.

#### Scenario: Cyclic graph does not loop infinitely
- **GIVEN** edges `(A, rel, B)` and `(B, rel, A)` exist
- **WHEN** `traverse("A", "rel", depth=5)` is called
- **THEN** the result contains `B` exactly once and the call terminates

#### Scenario: Depth limit is respected
- **GIVEN** a linear chain `A→B→C→D` with predicate `next`
- **WHEN** `traverse("A", "next", depth=2)` is called
- **THEN** the result contains `B` and `C` but NOT `D`

### Requirement: Wasmtime resource drop MUST release redb reader references
The `drop` implementation for `workspace-graph` SHALL delete the `WorkspaceGraphResource` from the host `ResourceTable`, ensuring any read transaction references are released and redb reader slots are not exhausted.

#### Scenario: Long-running FaaS invocations do not exhaust readers
- **GIVEN** many concurrent FaaS guests each open a `workspace-graph` handle
- **WHEN** each guest completes and drops its handle
- **THEN** the `ResourceTable` entry is removed and the redb reader count decreases
