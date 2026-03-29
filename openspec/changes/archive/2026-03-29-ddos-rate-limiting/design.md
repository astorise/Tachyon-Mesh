# Design: Change 018 - Compile-Time Anti-DDoS & Rate Limiting

## Summary
`core-host` gets an optional per-IP rate limiting layer that is only compiled when the `rate-limit` Cargo feature is enabled. The limiter sits in the Axum middleware stack before guest execution so rejected requests return immediately with HTTP `429 Too Many Requests`.

## Decisions

### Feature gating
- `core-host/Cargo.toml` exposes a `rate-limit` feature that enables the optional `governor` and `forwarded-header-value` dependencies.
- The rate limiter module and router wiring are guarded by `#[cfg(feature = "rate-limit")]`.
- When the feature is disabled, the router is built exactly as before with no extra middleware state.

### Client IP resolution
- Prefer the first IP from `X-Forwarded-For` when a trusted proxy forwards the original client identity.
- Fall back to Axum `ConnectInfo<SocketAddr>` so direct connections are still rate limited.
- If neither source is available, the request is allowed through instead of rejecting ambiguous traffic.

### Limiter shape
- Use a shared keyed `governor::RateLimiter` stored in an `Arc`.
- Key the limiter by `std::net::IpAddr`.
- Hardcode an MVP quota of `100` requests per second per IP as described in the proposal.
