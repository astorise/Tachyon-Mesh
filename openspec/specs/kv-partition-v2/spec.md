# kv-partition-v2 Specification

## Purpose
Native B-Tree key-value partition for FaaS guests: Component Model resource handles, IPC-optimised pagination, and atomic batch transactions.

## 1. The WIT Contract
Codex must update the WIT definition for the Key-Value partition capability to strictly match the following interface using the Component Model `resource` semantic.

```wit
package tachyon:ai@1.0.0;

interface kv-partition {
    resource table {
        /// Opens or creates a table in the current namespace's redb instance.
        constructor(name: string);

        get: func(key: string) -> result<list<u8>, string>;
        set: func(key: string, value: list<u8>) -> result<_, string>;
        delete: func(key: string) -> result<_, string>;
        
        /// Performs an atomic batch insert within a single write transaction.
        batch-set: func(entries: list<tuple<string, list<u8>>>) -> result<_, string>;

        /// Native B-Tree range query with pagination to prevent Wasm OOM.
        get-range: func(
            start-key: string, 
            end-key: string, 
            limit: u32, 
            offset: u32
        ) -> result<list<tuple<string, list<u8>>>, string>;
    }
}
```

## 2. Core-Host Wasmtime Integration
In `core-host/src/host_core/` (where the AI/KV bindings are implemented):
- The `TachyonCtx` (or equivalent Wasmtime `Store` state) must include a Wasmtime `ResourceTable`.
- Implementing the `resource` requires defining a Rust struct (e.g., `RedbTableResource`) that holds the table name and a reference to the active `redb::Database`.
- **`batch-set` Implementation:** Codex must open a `db.begin_write()`, iterate over the `entries`, insert them into the table, and then explicitly `.commit()`.
- **`get-range` Implementation:** Codex must open a `db.begin_read()`, use `.range(start_key..end_key)`, apply `.skip(offset as usize).take(limit as usize)`, and collect the results into the returned `Vec<(String, Vec<u8>)>`. 
- Provide detailed error mapping for Redb storage failures (e.g., corrupted database, serialization errors).

## 3. Rust FaaS SDK Refactoring
In `faas-sdk/src/`, Codex must update the API bindings so developers can use the resource ergonomically.

**Target SDK Usage Example for Codex to ensure:**
```rust
use tachyon_sdk::ai::kv_partition::Table;

pub fn execute_agent_logic() {
    // Constructor automatically maps to `open-table` resource creation
    let context_table = Table::new("viking-context");
    
    // Batch insert
    context_table.batch_set(&[
        ("node:1".to_string(), b"data1".to_vec()),
        ("node:2".to_string(), b"data2".to_vec()),
    ]).unwrap();
    
    // Paginated Range Query
    let history = context_table.get_range("timestamp:20260512T0000", "timestamp:20260512T2359", 50, 0).unwrap();
}
```
*Note: The Wasm runtime will automatically invoke the resource `drop` export when `context_table` goes out of scope, releasing host-side tracking.*