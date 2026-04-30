# Proposal: Modularization & Enterprise Testing

## Context
The `core-host` has grown into a 22,000+ lines monolith in `main.rs`. This hinders development velocity, slows down compilation, and makes pinpointing logic errors difficult. Furthermore, the current test coverage is insufficient for a security-critical service mesh. 

## Proposed Solution
1. **Structural Refactoring:** Break down `main.rs` into a hierarchical module system (`src/runtime/`, `src/network/`, `src/identity/`, etc.).
2. **State Decoupling:** Isolate `AppState` and `IntegrityRuntime` into a dedicated `state` module to clarify ownership.
3. **Enterprise Testing Suite:** Implement a mandatory testing policy:
   - **Unit Tests:** 80%+ coverage for business logic.
   - **Property-Based Testing:** Use `proptest` for input validation and parsers.
   - **Integration Tests:** Scenario-based tests in `core-host/tests/` simulating full node lifecycles.
   - **Mocking:** Create a formal trait-based system to mock System FaaS IPC calls.

## Objectives
- Reduce `main.rs` to a simple entry point (< 500 lines).
- Increase reliability and confidence for enterprise production deployments.
- Improve compilation times by enabling better crate-level parallelism.