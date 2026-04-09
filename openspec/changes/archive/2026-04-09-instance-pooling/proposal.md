# Proposal: Change 041 - FaaS Instance Pooling & Scaling Limits

## Context
WebAssembly instantiation is incredibly fast (microseconds), but memory is finite. An unbound influx of requests will cause the `core-host` to blindly instantiate thousands of WASM modules, leading to Memory Exhaustion (OOM). Furthermore, certain architectural patterns require strict concurrency limits (Singletons) or guaranteed baseline performance (Pre-warmed instances). The host must actively manage the lifecycle and scaling boundaries of each FaaS.

## Objective
Implement an asynchronous Instance Pool per FaaS target in the Rust `core-host`. 
1. Introduce `scale` configurations (`min` and `max`) in the `integrity.lock`.
2. The host will pre-instantiate `min` instances at startup.
3. The host will cap concurrent instances at `max`. 
4. If a request arrives and the `max` limit is reached, the host will place the request in a fast, in-memory asynchronous wait queue until an active instance finishes its execution and is returned to the pool.

## Scope
- Update `integrity.lock` schema to support the `scale` block.
- Implement an Object Pool pattern for Wasmtime instances in Rust.
- Introduce a request timeout mechanism so requests don't wait in the queue indefinitely if the FaaS instances are deadlocked.

## Success Metrics
- Setting `max: 1` guarantees that only one request is processed at a time for that specific FaaS (Singleton behavior).
- A load test of 10,000 concurrent requests against a target with `max: 100` results in exactly 100 active memory allocations, with the remaining requests successfully queued and processed sequentially without dropping, proving absolute memory protection.