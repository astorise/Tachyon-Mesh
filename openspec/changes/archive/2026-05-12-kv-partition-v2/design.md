# Design: KV-Partition V2 — Native Redb B-Tree, Resource Model, and IPC Optimization

## Approach

Three layers: WIT contract, host implementation, and guest SDK.

### 1. WIT Contract

The `kv-partition` interface is defined in two places:
- `wit/ai/kv-partition.wit` — canonical AI-package definition for documentation and guest SDK WIT tooling
- `wit/tachyon.wit` — inline copy consumed by `wasmtime::component::bindgen!` in `core-host/src/main.rs` (Wasmtime requires a single WIT file for its `bindgen!` macro input)

The interface uses the Component Model `resource table` semantic. The Wasm runtime calls the `drop` export automatically when the guest handle goes out of scope, so there is no explicit `close` function — a deliberate IPC reduction over v1's manual `u32` handle approach.

### 2. Host Implementation

**`RedbTableResource`** (`runtime_types.rs`) — the Rust struct stored inside Wasmtime's `ResourceTable`. Holds the table name string and an `Arc<CoreStore>` reference. Each guest-created `table` resource maps one-to-one with one of these; the `ResourceTable` gives lifetime safety through Wasmtime's `Resource<T>` ownership model.

**`HostTable` impl** (`component_hosts.rs`) — follows the same `self.table.push` / `self.table.get` pattern as the websocket `HostConnection` impl. Constructor: stores a `RedbTableResource` and returns an owned `Resource<Table>` with the same `rep()`. All method impls delegate to `CoreStore`.

**`CoreStore` additions** (`store/mod.rs`):
- `kv_partition_get` — single read transaction, opens table lazily (returns `None` if table doesn't exist)
- `kv_partition_set` — single write transaction, auto-creates the table on first write
- `kv_partition_delete` — removes key; no error if absent
- `kv_partition_batch_set` — single `WriteTransaction`, iterates entries, commits once → one fsync instead of N
- `kv_partition_get_range` — single read transaction, `table.range(start..end).skip(offset).take(limit)` → pagination is B-Tree-native; no full-table IPC transfer

All tables are namespaced as `kv_partition::{user_name}` inside redb, isolating them from internal system tables.

**Error mapping**: redb `TableError::TableDoesNotExist` is silently handled as an empty result; all other redb errors are surfaced as `anyhow::Error` and converted to `String` at the WIT boundary.

### 3. Rust SDK (`sdk/rust`)

`sdk/wit/tachyon.wit` gains the `kv-partition` interface and a new `world kv-consumer { import kv-partition; }` world. The minimal world avoids the handler export requirement, making it usable in library crates.

`sdk/rust/src/lib.rs` uses `wit_bindgen::generate!` with `kv-consumer` to generate the raw bindings, then exposes an ergonomic `ai::kv_partition::Table` wrapper that:
- Takes `&str` slices (not `String`) for keys
- Accepts `&[(String, Vec<u8>)]` for `batch_set` (matching common Rust idioms)
- Provides doc comments explaining pagination and auto-Drop semantics

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| WIT duplication | Inline copy in `wit/tachyon.wit` | Separate file include | Wasmtime `bindgen!` loads a single WIT file — multi-file WIT worlds require a WIT package directory, which is a larger refactor |
| Table naming | `kv_partition::{name}` prefix in redb | Separate redb database file per table | Shared DB = shared WAL / lock; simpler ops |
| Range iteration | `redb::ReadableTable::range(start..end).skip(offset).take(limit)` | Guest-side filtering | Pushes pagination to the B-Tree; eliminates the IPC tax of transferring large result sets |
| `ResourceTable` rep aliasing | `Resource::<RedbTableResource>::new_borrow(self_.rep())` | Separate HashMap by rep | Consistent with existing websocket pattern; redb is `Send + Sync` so no lock needed |
