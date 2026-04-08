# Tasks: Change 058 Implementation

**Agent Instruction:** Implement the `redb` embedded store. You MUST strictly separate synchronous database operations from the async Tokio runtime context to avoid blocking the network event loop. Use 4-space indentation.

## [TASK-1] Database Initialization
1. Add `redb = "2.0"` to `Cargo.toml`.
2. Create `src/store/mod.rs`. Define the `redb::TableDefinition` for `cwasm_cache`, `tls_certs`, and `hibernation_state`.
3. In the host bootstrap process, initialize the `redb::Database::create(path)` based on the `data_dir` config. Store the `Database` handle in an `Arc` so it can be shared across Tokio tasks.

## [TASK-2] Wasmtime Engine Cache Hook
1. Refactor the WASM instantiation logic.
2. Before calling `Engine::precompile_module()`, hash the `.wasm` file.
3. Open a `redb` read transaction. If the hash exists in `cwasm_cache`, load the bytes using `Module::deserialize()`.
4. If it's a miss, compile it. Then, use `tokio::task::spawn_blocking` to open a `redb` write transaction and insert the new `.cwasm` bytes into the table.

## [TASK-3] TLS & Hibernation Refactoring
1. Update the implementation of Change 057: The TLS RAM cache must be initially populated by reading the `tls_certs` table on startup. When `system-faas-cert-manager` returns a new cert, the host saves it to `redb` via `spawn_blocking`.
2. Update the implementation of Change 040: Instead of writing hibernated RAM to raw `.zip` files, insert the byte array directly into the `hibernation_state` table.