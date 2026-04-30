# Implementation Tasks

## Phase 1: Infrastructure and Linting
- [x] Add `thiserror = "1.0"` to `core-host/Cargo.toml`.
- [x] Create the `core-host/src/error.rs` file and define the base `TachyonError` enum.
- [x] Inject `#![warn(clippy::unwrap_used)]` at the top of `main.rs`. (Keep it as `warn` initially during the refactor, then switch to `deny` at the end).

## Phase 2: Strategic Replacement (Iterative)
Run `cargo clippy` and tackle the warnings module by module:
- [x] **Configuration & Boot:** Replace unwraps with graceful process exits (e.g., `eprintln!` and `std::process::exit(1)`) if the host cannot safely start.
- [x] **Network & Routing:** Replace unwraps with `Result` propagation. Ensure missing headers or bad payloads return HTTP 400.
- [x] **Wasm Runtime:** Catch `wasmtime` instantiation and memory errors and propagate them as HTTP 500.

## Phase 3: The Edge Cases
- [x] Use `.expect("...")` ONLY for absolute invariants (e.g., regex compilation constants or Mutex poisoning where recovery is impossible), but strictly document them with a `// SAFETY:` comment. If possible, avoid even this.
- [x] For `Option` types, convert them to Results using `.ok_or_else(|| ...)?`.

## Phase 4: CI/CD Validation
- [x] Change the top-level lint to `#![deny(clippy::unwrap_used)]`.
- [x] Ensure the GitHub Actions CI pipeline runs `cargo clippy -- -D clippy::unwrap_used` so that no future PR can introduce a panic.
