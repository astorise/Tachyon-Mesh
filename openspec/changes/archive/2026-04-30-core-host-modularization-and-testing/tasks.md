# Implementation Tasks

## Phase 1: Structural Refactoring
- [x] Create the sub-directories in `core-host/src/`.
- [x] Move logic from `main.rs` to modules (starting with `telemetry.rs` and `auth.rs`).
- [x] Refactor `AppState` and its related methods into `src/state/`.
- [x] Ensure all `pub(crate)` visibility modifiers are correctly set.

## Phase 2: Error Handling Audit
- [x] Run `cargo clippy -- -W clippy::unwrap_used -W clippy::expect_used`.
- [x] Replace `unwrap()` calls with proper `Result<T, CoreError>` propagation using the `thiserror` crate.

## Phase 3: Testing Infrastructure
- [x] Add `proptest`, `mockall`, and `tokio-test` to `dev-dependencies`.
- [x] Implement unit tests for the L4/L7 routing logic.
- [x] Implement a full integration test: "Deploy node -> Load FaaS -> Execute H3 request -> Verify Log".

## Phase 4: CI/CD & Coverage
- [x] Integrate a code coverage tool (e.g., Codecov) in the GitHub Actions workflow.
- [x] Update README with a "Testing Standards" section for future contributors.
