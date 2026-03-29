## ADDED Requirements

### Requirement: Host can enforce optional per-IP rate limiting at compile time
The `core-host` HTTP gateway SHALL expose a `rate-limit` Cargo feature that compiles in a shared per-IP rate limiting middleware while keeping the default build free of rate-limiting state when the feature is disabled.

#### Scenario: Feature is disabled
- **WHEN** `core-host` is built without `--features rate-limit`
- **THEN** the HTTP router is created without a rate limiting layer
- **AND** the default build carries no runtime rate limiting state

#### Scenario: Feature is enabled
- **WHEN** `core-host` is built with `--features rate-limit`
- **THEN** the HTTP router initializes a shared per-IP limiter with a quota of `100` requests per second
- **AND** requests are evaluated by that limiter before guest execution starts

### Requirement: Host rejects burst traffic with HTTP 429
When the `rate-limit` feature is enabled, the HTTP gateway SHALL resolve the client identity from `X-Forwarded-For` or the peer socket address and reject requests that exceed the configured quota with HTTP `429 Too Many Requests`.

#### Scenario: Same client exceeds the quota
- **WHEN** a single client IP sends `101` requests within one second
- **THEN** the first `100` requests are allowed to continue normally
- **AND** the `101st` request is rejected with HTTP `429 Too Many Requests`
- **AND** the rejection happens before the guest module runs
