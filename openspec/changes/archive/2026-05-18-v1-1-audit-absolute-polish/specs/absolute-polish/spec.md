# absolute-polish Specification Delta

## ADDED Requirements

### Requirement: CI Forbids Dead Code
The GitHub Actions workflow SHALL run Rust check/test steps with `RUSTFLAGS="-D dead_code"` so unused stubs cause an immediate CI failure.

#### Scenario: CI fails on dead code
- **GIVEN** a pull request introduces an unreferenced public function in `core-host`
- **WHEN** the `ci.yml` workflow runs `cargo check` or `cargo test`
- **THEN** the step SHALL fail because of `dead_code` being denied

### Requirement: BLAKE3 Safetensors Verification
The experimental QUIC safetensors replication path SHALL verify chunks with BLAKE3 instead of SHA-256.

#### Scenario: Verifier accepts a valid BLAKE3 chunk
- **GIVEN** a safetensors chunk and its BLAKE3 digest
- **WHEN** `verify_safetensors_blake3` is called
- **THEN** the verifier SHALL return success only when the recomputed BLAKE3 digest matches

#### Scenario: Verifier rejects a mismatched chunk
- **GIVEN** a safetensors chunk and an unrelated BLAKE3 digest
- **WHEN** the verifier evaluates the digest
- **THEN** the verifier SHALL return a verification error

### Requirement: Safetensors JSON Header Parsing
`core-host/src/ai_inference.rs` SHALL parse the safetensors header by reading the leading `u64` length prefix and decoding the JSON segment to extract `shape`, `dtype`, and `data_offsets`.

#### Scenario: Header is parsed before mapping
- **GIVEN** a safetensors file with a valid header
- **WHEN** `LayerWiseMappedModel` opens the file
- **THEN** the loader SHALL read the first 8 bytes as little-endian `u64`
- **AND** the loader SHALL parse the subsequent UTF-8 JSON segment
- **AND** the loader SHALL expose `shape`, `dtype`, and `data_offsets` to downstream callers

### Requirement: CDC and OLAP Boundary Lifecycle Tests
`core-host/tests/` SHALL contain `cdc_broadcaster_test.rs` and `olap_engine_test.rs` that instantiate the host components and verify a simulated Wasmtime boundary connection without panicking.

#### Scenario: cdc_broadcaster_test loads the component
- **GIVEN** the `cdc_broadcaster_test` integration test
- **WHEN** the test runs
- **THEN** the cdc-broadcaster component SHALL initialize and respond to a simulated host call without panicking

#### Scenario: olap_engine_test enforces payload limits
- **GIVEN** the `olap_engine_test` integration test
- **WHEN** the test submits an oversized payload
- **THEN** the olap-engine SHALL refuse the payload per the v1.1 memory bound
