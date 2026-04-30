# Design: Composite CRDT Keys

## 1. Schema Update
```json
"policies": {
  "ai-tier-basic": {
    "limit": 100,
    "window": "1m",
    "scope": "tenant"
  }
}
```

## 2. Dispatcher Logic (`core-host/src/network/router.rs`)
```rust
let limit_key = match policy.scope {
    LimitScope::Ip => req.peer_addr().ip().to_string(),
    LimitScope::Tenant => {
        let claims = req.extensions().get::<CallerIdentityClaims>()
            .ok_or(TachyonError::Unauthorized)?;
        format!("tenant:{}", claims.tenant_id)
    }
};

if !rate_limiter.check_and_increment(&limit_key, &policy).await {
    return Ok(Response::http_429_too_many_requests());
}
```