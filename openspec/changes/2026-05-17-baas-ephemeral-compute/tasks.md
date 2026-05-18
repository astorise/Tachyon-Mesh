# Implementation Tasks

- [x] **Task 1: Gateway Routing Logic**
  - Update `system-faas-gateway` to parse HTTP `Range` headers and introspect query payloads (looking for `GROUP BY`, `SUM`, or OLAP-specific Datalog traits) to trigger the ephemeral FaaS routing.

- [x] **Task 2: Media Streaming WIT & Host Implementation**
  - Define `wit/storage/media.wit`.
  - Implement `pipe-range-to-socket` in the `core-host`. This MUST utilize zero-copy mechanisms (e.g., Linux `sendfile` or mapped slices over QUIC streams) to move data from RustFS to the NIC without allocating large `Vec<u8>` buffers.

- [x] **Task 3: Implement `system-faas-media-server`**
  - Create the new Wasm component.
  - Implement HTTP `206 Partial Content` response formatting.
  - Parse the incoming `Range` header, calculate boundaries, and invoke `pipe-range-to-socket`.

- [ ] **Task 4: Implement `system-faas-olap-engine`**
  - Create the new Wasm component.
  - Implement a basic columnar aggregator in Rust (compiled to Wasm) that processes streamed dataset chunks to execute analytical functions safely isolated from the `core-host` memory space.
