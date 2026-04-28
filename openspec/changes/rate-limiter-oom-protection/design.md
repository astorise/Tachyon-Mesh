# Design: Bounded LRU Implementation

## 1. Dependency Update (`core-host/Cargo.toml`)
If not already present, we will rely on a high-performance concurrent cache crate such as `moka`. It provides highly concurrent LRU caching out of the box.

```toml
[dependencies]
moka = { version = "0.12", features = ["sync"] }
```

## 2. Refactoring the State (`core-host/src/rate_limit.rs`)
Replace the existing map with a `moka::sync::Cache` (or `moka::future::Cache` if strictly async context is required).

### Cache Initialization
The rate limiter state initialization must define explicit limits:
```rust
use moka::sync::Cache;
use std::time::Duration;

let ip_cache: Cache<String, RateLimitState> = Cache::builder()
    // Hard limit to prevent OOM (100k IPs is roughly ~10MB of RAM)
    .max_capacity(100_000)
    // Automatically evict stale trackers
    .time_to_idle(Duration::from_secs(60))
    .build();
```

## 3. Rate Limiting Logic
The evaluation logic remains mostly the same, but interacts with the bounded cache:
1. `ip_cache.get_with(ip, || RateLimitState::new())` (or equivalent `get`/`insert` flow).
2. Increment the request count.
3. If the count exceeds the threshold, return an HTTP `429 Too Many Requests`.
4. The LRU mechanics (eviction) are handled entirely and transparently by the `moka` background thread.