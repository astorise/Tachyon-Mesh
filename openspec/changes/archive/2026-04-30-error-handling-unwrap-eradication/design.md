# Design: Unified Error Architecture

## 1. Compiler Enforcement (`core-host/src/lib.rs` or `main.rs`)
At the very top of the crate root, enforce the linter rules to ensure no new unwraps are merged into the codebase.

```rust
#![deny(clippy::unwrap_used)]
#![deny(clippy::expect_used)]
#![deny(clippy::panic)]
```
*Note: We may selectively allow `unwrap()` in test modules using `#[cfg(test)] #![allow(clippy::unwrap_used)]`.*

## 2. Centralized Error Enum (`core-host/src/error.rs`)
Use `thiserror` to define a single, rich error type for the core host.

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TachyonError {
    #[error("Failed to load configuration: {0}")]
    ConfigError(String),

    #[error("Wasm execution trapped or failed: {0}")]
    WasmExecutionError(#[from] wasmtime::Trap),

    #[error("I/O Error occurred: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Missing expected header: {0}")]
    MissingHeader(&'static str),
    
    // ... add more variants as refactoring dictates
}

pub type Result<T> = std::result::Result<T, TachyonError>;
```

## 3. Refactoring Strategy (The `?` Operator)
Instead of extracting values forcefully:
```rust
// BAD (Current)
let header = req.headers().get("Authorization").unwrap();

// GOOD (Proposed)
let header = req.headers().get("Authorization")
    .ok_or(TachyonError::MissingHeader("Authorization"))?;
```
All functions in the request execution path must return `crate::error::Result<T>`. At the top level (the HTTP router/dispatcher), map these errors to standard HTTP response codes.