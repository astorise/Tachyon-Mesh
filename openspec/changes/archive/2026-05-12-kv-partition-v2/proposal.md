# Title: KV-Partition V2: Native Redb B-Tree, Resource Model, and IPC Optimization

## Problem Statement
The current `kv-partition` WIT interface uses a flat key space and relies on transferring large datasets across the Wasm-Host boundary (IPC tax) for guest-side filtering. Furthermore, using manual `u32` handles for tables risks memory leaks and invalid concurrent accesses. Single `set` operations introduce massive `fsync` overhead for AI agents inserting complex knowledge graphs.

## Objective
Upgrade the `tachyon:ai/kv-partition` WIT contract and its host implementation to v2:
1. **Component Model Resources:** Use WIT `resource table` for memory-safe, object-oriented table handles with auto-Drop capabilities.
2. **IPC Optimization & Anti-OOM:** Implement `get-range` with pagination (`limit`, `offset`) delegated to Redb's native B-Tree iterators.
3. **Transaction Batching:** Introduce `batch-set` to allow atomic, multi-key inserts within a single Redb transaction.