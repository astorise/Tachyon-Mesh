# Implementation Tasks

## Phase 1: Code Deletion
- [ ] Delete the `turboquant-sys` folder entirely from the project root.
- [ ] Remove `turboquant-sys` from the workspace `Cargo.toml` members.

## Phase 2: Dependency Injection
- [ ] Identify which module previously consumed `turboquant-sys` (likely `core-host`).
- [ ] Add the native Rust `turboquant` crate to that module's `Cargo.toml` dependencies.

## Phase 3: Rust Implementation
- [ ] Replace all `unsafe { turboquant_c_api(...) }` calls with the safe equivalents from the native crate.
- [ ] Ensure the quantization parameters (bits, dimensions) match the previous C++ behavior to maintain model compatibility.

## Phase 4: Validation
- [ ] Run `cargo build` on a clean environment to ensure no C++ compiler is invoked.
- [ ] Run the AI inference tests to verify that KV cache compression and generation perplexity remain identical to the legacy implementation.