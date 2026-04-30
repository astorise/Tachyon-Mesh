# Proposal: Core-Host Modularization

## Context
The `core-host/src/main.rs` file has grown to an unmanageable size (> 23,000 lines). A previous attempt to refactor this failed, resulting in only placeholder directories being created while the actual code remained in `main.rs`. This monolithic structure cripples developer velocity, massively increases compilation times, and hides 11 remaining production `panic!` calls.

## Proposed Solution
We must execute a strict, phased extraction of logic from `main.rs` into dedicated submodules.
The target architecture for `core-host/src/` is:
- `main.rs`: Entry point and CLI parsing ONLY.
- `state.rs` (or `state/mod.rs`): Global `AppState`, `RuntimeState`, and shared `ArcSwap` configurations.
- `network/`: Handling HTTP/3, routing, L4/L7 logic, and connection management.
- `runtime/`: Wasmtime engine initialization, FaaS loading, caching, and pooling.
- `telemetry.rs` (or `telemetry/mod.rs`): Logging setup and metrics (if not fully moved to `system-faas-otel`).
- `identity/`: Authentication, authorization, and JWT parsing.

## Objectives
- Reduce `main.rs` to under 1,000 lines.
- Isolate the 11 remaining `panic!` calls during the extraction and replace them with proper error handling.
- Establish a scalable module tree that makes future feature additions trivial.