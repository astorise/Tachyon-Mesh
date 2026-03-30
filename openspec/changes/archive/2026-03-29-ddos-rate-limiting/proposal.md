# Proposal: Change 018 - Compile-Time Anti-DDoS & Rate Limiting

## Context
Exposing a FaaS platform directly to the internet makes it vulnerable to DDoS attacks or abusive clients that exhaust CPU resources and Wasmtime execution pools. We need a robust Rate Limiting mechanism. However, aligned with Tachyon's "Zero-Overhead" philosophy, this feature must cost exactly 0 bytes and 0 CPU cycles if the platform is deployed behind an external Load Balancer/CDN that already handles rate limiting.

## Objective
Implement an IP-based Rate Limiting middleware in the `core-host` using the highly optimized `governor` crate. This middleware will be gated behind a Rust compile-time feature flag (`rate-limit`). When enabled, it will track incoming requests per IP and reject those exceeding a defined quota with an HTTP 429 (Too Many Requests).

## Scope
- Add `governor` and `forwarded-header-value` as optional dependencies in `core-host/Cargo.toml`.
- Create an Axum middleware that extracts the client's IP (falling back to `X-Forwarded-For` if behind a proxy).
- Apply a Token Bucket quota (e.g., 100 requests per second per IP).
- Wrap the entire middleware registration in `#[cfg(feature = "rate-limit")]` to ensure Dead Code Elimination when the feature is disabled.
- Return a strict `HTTP 429 Too Many Requests` when the bucket is empty.

## Success Metrics
- Compiling `core-host` without the feature flag produces a binary with zero rate-limiting overhead.
- Compiling with `--features rate-limit` enables the middleware.
- A single IP sending 101 requests within a second receives an HTTP 429 on the 101st request, preventing Wasmtime instantiation.