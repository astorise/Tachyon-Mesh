# Implementation Tasks

- [ ] **Task 1: Complete Integration Test Suite**
  - Scaffold `core-host/tests/view_builder_test.rs` with a basic host-guest instantiation test.
  - Scaffold `core-host/tests/sql_engine_test.rs` with a basic instantiation test.
  - Scaffold `core-host/tests/vector_search_test.rs` with a basic instantiation test.
  - Scaffold `core-host/tests/media_server_test.rs` with a basic instantiation test.

- [x] **Task 2: Purge Phantom Dependencies**
  - Open `core-host/Cargo.toml`.
  - Locate and remove `biscuit-auth` from the default dependencies (or move it under the `experimental` feature gate).

- [x] **Task 3: Delete Broken Constrained Decoding Stubs**
  - Clean `core-host/src/ai_inference/samplers.rs` of the broken dummy FSM logic flagged by the audit.
  - Remove the constrained decoding WIT definitions from the `wit/` directory.
