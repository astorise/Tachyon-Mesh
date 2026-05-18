# Implementation Tasks

- [x] **Task 1: Enforce `-D dead_code` in CI**
  - Open `.github/workflows/ci.yml` (and `integration.yml` / `e2e.yml` if applicable).
  - Inject `RUSTFLAGS: "-D dead_code"` into the `env` block of the formatting, linting, and testing steps.

- [x] **Task 2: Implement BLAKE3 for QUIC Safetensors**
  - Add `blake3` to `core-host/Cargo.toml`.
  - In `core-host/src/server_h3.rs`, rewrite the hashing logic to utilize `blake3` instead of `sha2` / `sha256`.

- [x] **Task 3: Implement Safetensors Header Parsing**
  - In `core-host/src/ai_inference.rs`, write the logic to read the first 8 bytes (as little-endian u64) to get the JSON header length.
  - Parse the subsequent bytes as a JSON string to extract `shape`, `dtype`, and `data_offsets`.

- [x] **Task 4: Expand Integration Test Suite**
  - Create `core-host/tests/cdc_broadcaster_test.rs` and write a basic lifecycle test.
  - Create `core-host/tests/olap_engine_test.rs` and write a basic lifecycle test validating the memory-bound JSON payload limits implemented in Phase 1.
