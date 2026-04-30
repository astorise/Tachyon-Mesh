# Proposal: Eradication of Panicking Unwraps

## Context
A static analysis of the `core-host` repository revealed approximately 740 instances of `unwrap()`, `expect()`, and `panic!()`. While acceptable in prototypes or CLI tools, these macros induce immediate process termination (panics) upon encountering unexpected data or state. For a Service Mesh acting as the network backbone (routing HTTP/3, invoking AI models, managing IPC), a single malformed packet or missing file could crash the entire host, terminating all other active connections. This is a critical operational risk.

## Proposed Solution
We will implement a **Zero-Panic Policy** for the runtime:
1. **Linter Enforcement:** Strictly forbid `unwrap_used` and `expect_used` at the compiler level via `clippy`.
2. **Domain Errors:** Introduce a centralized error system using the `thiserror` crate (`core-host/src/error.rs`) to elegantly categorize and format all possible failure modes (e.g., `WasmError`, `NetworkError`, `ConfigError`).
3. **Graceful Degradation:** Replace panics with proper `Result<T, TachyonError>` returns. If an individual request or FaaS invocation fails, the host will trap the error, log it, return an HTTP 500/502 to the client, and continue serving other traffic seamlessly.

## Objectives
- Achieve 100% uptime resilience against malformed inputs or localized component failures.
- Greatly improve debugging by providing contextual error messages instead of generic "called Option::unwrap() on a None value" panics.
- Make error handling idiomatic Rust (using the `?` operator).