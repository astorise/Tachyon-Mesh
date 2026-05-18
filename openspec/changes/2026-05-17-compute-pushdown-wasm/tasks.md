# Implementation Tasks

- [x] **Task 1: Define Filter WIT**
  - Create `wit/storage/pushdown-filter.wit`.

- [x] **Task 2: RedDB Execution Sandbox**
  - Implement a highly optimized, pooled Wasmtime `Engine` inside the `core-host`'s RedDB module.
  - Configure Wasmtime `Config::consume_fuel(true)` and strict memory limits (e.g., max 1MB per filter instance) to guarantee storage node stability.

- [x] **Task 3: RedDB API Extension**
  - Update the `tachyon:storage/reddb-direct` WIT contract so FaaS modules can pass the `pushdown_filter_wasm` byte array alongside standard `scan` requests.

- [ ] **Task 4: Proof of Concept (Logical Plane)**
  - Update a single FaaS (e.g., `system-faas-sql-engine`) to pass a pre-compiled dummy filter (e.g., a filter that only returns true if the key ends with a specific byte) to validate the network I/O reduction metrics.
