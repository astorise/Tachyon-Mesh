# Design: Engine & Pool Architecture

## 1. Engine Configuration (`core-host/src/store/mod.rs` or Engine initialization)
The Wasmtime engine must be reconfigured to use the pooling allocator.

```rust
use wasmtime::{Engine, Config, InstanceAllocationStrategy, PoolingAllocationConfig};

let mut pool_config = PoolingAllocationConfig::default();
// Configure the maximum number of concurrent instances
pool_config.total_core_instances(10_000);
// Configure limits per instance (e.g., 128MB max per Wasm module)
pool_config.max_memory_size(128 * 1024 * 1024);

let mut config = Config::new();
config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool_config));
config.async_support(true);

let engine = Engine::new(&config).expect("Failed to create pooled Wasmtime Engine");
```

## 2. The Pre-compilation Cache
Create a module cache mechanism within the `core-host` state:
- **Structure:** `Cache<String, wasmtime::InstancePre<StoreState>>` (keyed by the module's hash or registry ID).
- **Miss Flow:** If the cache misses, fetch the blob, compile to `Module`, link imports via `Linker`, call `linker.instantiate_pre(&module)`, and store the resulting `InstancePre` in the cache.
- **Hit Flow:** If the cache hits, clone the `InstancePre` (which is cheap, as it's an `Arc` internally).

## 3. Request Execution
The FaaS request handler now simply does:
```rust
let instance_pre = module_cache.get(&module_id).await?;
let mut store = Store::new(&engine, state);
let instance = instance_pre.instantiate_async(&mut store).await?;
// Proceed with function execution...
```