# Semantic Graph Engine Specifications

## 1. WIT Contract Definition
Codex must create `wit/store/graph.wit` with the following strict contract:
```wit
package tachyon:store@1.0.0;

interface graph {
    record edge {
        subject: string,
        predicate: string,
        object: string,
        properties: string, // Strict JSON string
    }

    resource workspace-graph {
        constructor(name: string);
        
        add-edges: func(edges: list<edge>) -> result<_, string>;
        delete-edges: func(edges: list<edge>) -> result<_, string>;
        
        /// Performs a multi-hop traversal up to `depth`.
        traverse: func(subject: string, predicate: string, depth: u32) -> result<list<string>, string>;
    }
}
```

## 2. Redb Hexastore Schema & Storage Logic
Codex must implement the graph indices on top of `redb`.
Do NOT import external graph databases.

**Index Tables:**
For a given graph namespace (e.g., `viking`), define two `redb` tables:
1. `graph_{name}_spo`: Key is `(Subject, Predicate, Object)`, Value is `Properties` (JSON).
2. `graph_{name}_osp`: Key is `(Object, Subject, Predicate)`, Value is `Properties`.
*Note: Serialize the composite keys using null-byte (`\0`) separators for efficient prefix scanning in Rust.*

**Mutation (`add-edges` / `delete-edges`):**
- Open a `WriteTransaction`.
- For each edge, construct the `S\0P\0O` and `O\0S\0P` keys.
- Insert (or remove) the records atomically across both tables.
- Call `.commit()`.

## 3. Host-Side Traversal Algorithm
Codex must implement the `traverse(subject, predicate, depth)` logic in Rust to avoid sending the entire graph to Wasm.

**Algorithm:**
1. Initialize a `HashSet<String>` for `visited` nodes to prevent cyclic infinite loops.
2. Initialize a queue/vector with `(current_subject, current_depth = 0)`.
3. Open a `ReadTransaction` on the `SPO` table.
4. While the queue is not empty:
   - Pop `node`. If `current_depth == target_depth`, add to results and continue.
   - Use `redb`'s `.range()` iterator with a prefix scan on `node\0predicate\0` to efficiently find all outgoing objects.
   - For each extracted `object`:
     - If not in `visited`: insert into `visited` and push `(object, current_depth + 1)` to the queue.
5. Return the final list of collected objects. Ensure the Wasm return limit is respected (e.g., max 10,000 items) to prevent OOM.

## 4. Wasmtime Resource Integration
- Use `wasmtime::component::ResourceTable` in the host state to track open graph handles.
- Ensure the `drop` method automatically releases any underlying read locks to prevent `redb` reader exhaustion.