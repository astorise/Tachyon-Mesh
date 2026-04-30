# Design: New Module Architecture & Testing Strategy

## 1. Target Directory Structure
The new `core-host/src/` will follow this layout:
- `mod.rs` / `main.rs`: Entry point, CLI parsing, and top-level task orchestration.
- `runtime/`: Wasmtime engine setup, instance pooling, and hibernation.
- `network/`: HTTP/3 (H3) server, Layer 4/7 routing logic.
- `identity/`: IAM, AuthN/AuthZ, token validation.
- `storage/`: Volume management, VFS, and KV store drivers.
- `telemetry/`: Logging, OTLP tracing, and metering.
- `state/`: Global configuration, `ArcSwap` management.
- `mesh/`: Gossip protocol and P2P overlay orchestration.

## 2. Testing Standards
Every new module MUST include a `tests` sub-module or a sibling `.rs` file in `tests/`.

### PropTest for Parsers
For the `integrity.lock` parser and IPC payloads:
```rust
proptest! {
    #[test]
    fn test_config_parsing_robustness(s in "\\PC*") {
        let _ = parse_config(&s); // Should never panic
    }
}
```

### Integration Test Harness
A shared test helper to spin up a mock Tachyon environment:
```rust
pub async fn setup_test_node() -> (TestNode, MockFaaSChannel) {
    // Boilerplate to init logging, ephemeral DB, and a dummy runtime
}
```

## 3. CI Integration
Update `.github/workflows/ci.yml` to:
1. Run `cargo tarpaulin` or `llvm-cov` to generate coverage reports.
2. Fail the PR if coverage drops below the defined threshold for the modified modules.