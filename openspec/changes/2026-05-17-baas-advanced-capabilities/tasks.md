# Implementation Tasks

- [ ] **Task 1: CDC and Pub/Sub Broadcaster**
  - Implement `wit/storage/data-events.wit`.
  - Create the `system-faas-cdc-broadcaster` component. It must manage active QUIC/WebSocket streams and evaluate the embedded Biscuit token of each subscriber before forwarding a `mutation-event`.

- [ ] **Task 2: Ephemeral Vector Search**
  - Update RustFS to support pure binary contiguous blocks for embeddings.
  - Implement `system-faas-vector-search`. Include a highly optimized cosine similarity loop (ensure Wasm SIMD `wasm32-simd128` target features are enabled during compilation).

- [ ] **Task 3: Gossip CRDT Resolution Hook**
  - Implement `wit/storage/conflict-resolution.wit`.
  - Update `core-host/src/mesh/gossip.rs`. Inject the split-brain detection logic (comparing vector clocks).
  - Wire the host to instantiate the correct FaaS module to resolve the conflict automatically before finalizing the RedDB commit.
