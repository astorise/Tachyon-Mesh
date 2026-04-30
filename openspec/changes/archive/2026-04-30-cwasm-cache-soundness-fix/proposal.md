# Proposal: Cwasm Cache Soundness & UB Prevention

## Context
Tachyon Mesh uses `redb` to cache precompiled Wasm modules (Cwasm) to achieve sub-millisecond cold starts. Currently, the cache key is derived solely from the module's origin (`kind:scope:path:sha256(wasm)`). When the `core-host` is upgraded to a new version of `wasmtime`, or if the `wasmtime::Config` changes (e.g., enabling Fuel or a new WASI proposal), the host will attempt to load Cwasm compiled by an older engine via `unsafe { Component::deserialize }`. This is mathematically unsound and guarantees Undefined Behavior (UB) and memory corruption.

## Proposed Solution
We will leverage Wasmtime's built-in safeguard: `Engine::precompile_compatibility_hash()`.
1. **Cache Key Salting:** We will inject this hash into the `cache_key` generation logic. A cache entry will now strictly bind to the exact engine version and configuration that produced it.
2. **Stale Cache Eviction (Garbage Collection):** To prevent the `redb` database from growing indefinitely with orphaned Cwasm binaries after host upgrades, we will implement a startup routine that checks a global `last_engine_hash` key. If the hash differs, the entire Cwasm cache bucket is flushed cleanly before the router starts accepting traffic.

## Objectives
- Eliminate the risk of UB and segfaults related to Wasmtime engine mismatches.
- Ensure 100% safe host upgrades without requiring manual cache deletion.
- Maintain disk space hygiene by automatically purging obsolete compiled binaries.