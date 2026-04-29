# Proposal: Wasm Instance Pooling & Pre-compilation

## Context
Tachyon Mesh executes FaaS logic by instantiating WebAssembly modules inside the `core-host`. Currently, if the host compiles the Wasm binary and creates a new memory and table layout for every incoming request, it incurs a massive latency penalty (Cold Start). Under high concurrency, this CPU-bound compilation process will bottleneck the host, severely degrading the HTTP/3 network throughput.

## Proposed Solution
We will implement an **Instance Pooling Pattern** leveraging Wasmtime's advanced memory management:
1. **Pooling Allocator:** The Wasmtime `Engine` will be initialized with a `PoolingAllocationConfig`. This pre-allocates a fixed block of RAM for Wasm linear memories, avoiding expensive OS-level allocations (like `mmap`) on every request.
2. **Pre-compilation (`InstancePre`):** When a module is loaded from the `system-faas-registry`, it will be compiled exactly once into a `wasmtime::Module` and then into an `InstancePre`.
3. **Warm Cache:** These `InstancePre` objects will be stored in a concurrent LRU cache (e.g., `moka`). 
4. **Execution:** Incoming requests will simply fetch the `InstancePre` from the cache and call `.instantiate()`, which pulls from the pre-allocated pool in a fraction of a millisecond.

## Objectives
- Drop FaaS invocation latency to < 1ms (sub-millisecond warm starts).
- Prevent CPU spikes during high-concurrency traffic bursts.
- Cap the maximum memory used by the Wasm runtime to a predictable limit.