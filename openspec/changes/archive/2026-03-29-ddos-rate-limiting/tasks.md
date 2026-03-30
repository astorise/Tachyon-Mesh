# Tasks: Change 018 Implementation

## 1. OpenSpec Artifacts

- [x] 1.1 Add a delta spec under `openspec/changes/ddos-rate-limiting/specs/http-routing/spec.md` covering feature-gated per-IP rate limiting and HTTP `429` responses.
- [x] 1.2 Add `design.md` capturing feature gating, middleware placement, and client IP resolution.

## 2. core-host Implementation

- [x] 2.1 Add the `rate-limit` feature and optional dependencies to `core-host/Cargo.toml`.
- [x] 2.2 Implement a shared `governor`-backed rate limiting middleware that resolves the client IP from `X-Forwarded-For` or `ConnectInfo<SocketAddr>`.
- [x] 2.3 Wire the middleware into the Axum router only when the feature is enabled, and serve the live app with peer-address connect info.

## 3. Verification

- [x] 3.1 Add feature-gated regression coverage for proxy IP resolution and HTTP `429` behavior.
- [x] 3.2 Verify with `openspec validate --all`, `cargo test -p core-host --features rate-limit`, and `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
