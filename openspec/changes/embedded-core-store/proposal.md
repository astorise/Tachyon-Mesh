# Proposal: Change 058 - Embedded Core KV Store (`redb`)

## Context
In its V1 containerized deployment, the Tachyon `core-host` acts as a stateless entity by default. If the container restarts, the host loses compiled Wasmtime modules (`.cwasm`), active Let's Encrypt TLS certificates, and suspended RAM volumes of hibernating FaaS. Relying on an external database (like Redis or etcd) violates our "Single Binary" microkernel philosophy. Relying on raw filesystem writes introduces corruption risks during crashes.

## Objective
1. Integrate `redb`, a pure-Rust, memory-mapped, ACID-compliant B-Tree Key-Value store, directly into the `core-host`.
2. Use this embedded database as the ultra-fast persistent cache for internal node operations, ensuring crash-resilience.
3. Consolidate WASM compilation cache, TLS certificate storage, and RAM hibernation states into structured database tables rather than scattered loose files.

## Scope
- Add `redb` as a core dependency.
- Implement a `CoreStore` module in the host with specific Table definitions.
- Modify the startup sequence to open or create `tachyon.db`.
- Ensure all synchronous database write operations are wrapped in Tokio's `spawn_blocking` to prevent starving the async executor.