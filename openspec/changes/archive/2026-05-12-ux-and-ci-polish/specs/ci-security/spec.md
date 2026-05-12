## ADDED Requirements

### Requirement: CI validates cross-layer admin contracts
The CI workflow SHALL run a repository script that verifies Tachyon client admin endpoint constants are backed by matching core-host admin routes before build-heavy jobs proceed.

#### Scenario: Client admin endpoint has a host route
- **GIVEN** `tachyon-client/src/lib.rs` defines an `ADMIN_*_PATH` constant
- **WHEN** CI runs cross-layer validation
- **THEN** `scripts/validate_cross_layer.sh` verifies that `core-host/src/host_core/app_runtime.rs` contains the exact route literal or a dynamic route beneath that path

#### Scenario: Client admin endpoint is missing from the host
- **GIVEN** `tachyon-client/src/lib.rs` defines an `ADMIN_*_PATH` constant with no matching host route
- **WHEN** CI runs cross-layer validation
- **THEN** the script exits non-zero
- **AND** prints the missing endpoint to standard error

### Requirement: CI verifies Stronghold is actively used when declared
The CI workflow SHALL fail when the Tauri UI declares `tauri-plugin-stronghold` without active `Stronghold::` API usage in the Tauri Rust entrypoint.

#### Scenario: Stronghold dependency has active usage
- **GIVEN** `tachyon-ui/Cargo.toml` declares `tauri-plugin-stronghold`
- **WHEN** CI runs cross-layer validation
- **THEN** the script verifies `tachyon-ui/src/main.rs` contains a `Stronghold::` invocation

#### Scenario: Stronghold dependency is unused
- **GIVEN** `tachyon-ui/Cargo.toml` declares `tauri-plugin-stronghold`
- **AND** `tachyon-ui/src/main.rs` does not contain a `Stronghold::` invocation
- **WHEN** CI runs cross-layer validation
- **THEN** the script exits non-zero
