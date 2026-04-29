# Design: Native Rust Integration

## 1. Dependency Management (`Cargo.toml`)
Identify the module consuming the quantization (e.g., `core-host` or a specific FaaS) and add the native Rust implementation:
```toml
[dependencies]
turboquant = "0.2"
```

## 2. Removing the C++ Bridge
The entire `turboquant-sys` directory MUST be deleted from the repository. This includes:
- `build.rs` (C++ compiler invocations).
- `native/turboquant.cpp` and `native/turboquant.hpp`.
- Unsafe Rust FFI wrappers in `src/lib.rs`.

## 3. Inference Refactoring
In the modules handling the KV Cache, replace the unsafe calls with the new native structures.

```rust
use turboquant::QuantizedKVCache; // Example import from the native crate

// The logic becomes entirely safe Rust without FFI
let quantized_cache = QuantizedKVCache::new(...);
```