# Implementation Tasks

- [x] **Task 1: Biscuit Auth & SmolVM Integration**
  - Integrate the `biscuit-auth` crate in the `core-host`.
  - Establish the VSOCK proxy logic routing raw database queries to the domain-specific `ast-mutation` SmolVM instance.
  - Implement the `Query + Datalog` LRU cache in the `core-host` to maintain microsecond latency.

- [x] **Task 2: Internal WIT Data Bindings**
  - Create strongly-typed WIT bindings (`tachyon:storage/reddb-direct`) that allow internal Wasm components to perform `get`, `put`, and `scan` operations directly on RedDB, bypassing the Gateway and AST layers completely.

- [x] **Task 3: RustFS Media & Streaming**
  - Implement the `should_compress_blob` Magic Byte detector in the RustFS ingestion pipeline.
  - Create a dedicated `system-faas-media-server` component capable of processing `Range: bytes=X-Y` HTTP headers and streaming direct chunks from RustFS `mmap` to the network interface via QUIC/TCP.

- [x] **Task 4: Write-on-Read Shim Logic**
  - Implement the `schema-shim` WIT interface.
  - Modify the RedDB retrieval loop in `core-host`: if a returned record's version header is `< CURRENT_VERSION`, dispatch the payload to the registered Wasm shim for dynamic transformation before returning it to the user.
