# Design: semantic-graph-store

## Overview

Adds a host-side semantic graph store (hexastore) backed by redb, exposed to Wasm guests as a WIT `graph::workspace-graph` resource. Multi-hop BFS traversal runs entirely on the host to avoid serialising the whole graph into Wasm memory.

## Task 1 — WIT Contract (`wit/store/graph.wit` + `wit/tachyon.wit`)

`wit/store/graph.wit` declares `package tachyon:mesh@1.0.0` with an `interface graph` containing:
- `record edge { subject, predicate, object, properties }` — properties is a strict JSON string
- `resource workspace-graph` with constructor, `add-edges`, `delete-edges`, `traverse`

The same interface is added inline to `wit/tachyon.wit` (single-file package pattern used by the codebase) and imported into the `faas-guest` world alongside `kv-partition`.

## Task 2 — Redb Hexastore Schema

Two dynamically-named redb tables per graph namespace:
- `graph_{name}_spo` — `TableDefinition<&[u8], &str>` keyed by `S\0P\0O`
- `graph_{name}_osp` — `TableDefinition<&[u8], &str>` keyed by `O\0S\0P`

Composite keys use null-byte (`\0`) separators for efficient byte-ordered prefix scanning. Table names are constructed at runtime from the graph namespace string. Tables are opened lazily (redb creates them on first write).

## Task 3 — Mutation Logic (`store/mod.rs`)

`CoreStore::graph_add_edges` and `graph_delete_edges`:
1. Open a single `WriteTransaction`.
2. For each edge, compute the `spo_key` and `osp_key` vectors.
3. Insert/remove records atomically across both tables within the same transaction.
4. Call `.commit()`.

## Task 4 — Traversal Algorithm (`store/mod.rs`)

`CoreStore::graph_traverse(graph_name, subject, predicate, depth)`:
1. Initialise a `HashSet<String>` for visited nodes (seeded with `subject`).
2. BFS queue of `(node: String, current_depth: u32)`.
3. Open a `ReadTransaction` on the SPO table.
4. For each dequeued node where `current_depth < depth`:
   - Compute prefix `node\0predicate\0` and call `spo.range(start..end)`.
   - Extract the object from the key suffix (after stripping the prefix).
   - If not visited: insert into visited, append to results, enqueue with `depth+1`.
5. Hard cap at `GRAPH_TRAVERSE_LIMIT = 10_000` objects to prevent OOM.

## Task 5 — Wasmtime Bindings

`graph_store.rs` defines `WorkspaceGraphResource { graph_name, core_store }`.

`component_hosts.rs` implements `HostWorkspaceGraph for ComponentHostState`:
- `new` — pushes a `WorkspaceGraphResource` into `self.table` (the Wasmtime `ResourceTable`), returns a typed `Resource<WorkspaceGraph>`.
- `add_edges`, `delete_edges`, `traverse` — borrow the resource from the table, delegate to `CoreStore` methods.
- `drop` — deletes the resource from the table, releasing any held references so redb readers are not exhausted.
