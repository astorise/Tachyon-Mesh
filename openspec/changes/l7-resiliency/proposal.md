# Proposal: Change 028 - Compile-Time L7 Resiliency (Timeouts, Retries)

## Context
To match Istio's feature set and guarantee enterprise-grade reliability, Tachyon Mesh must protect clients from transient network failures and slow FaaS modules. However, implementing State Machines for Retries and Circuit Breakers introduces unavoidable CPU and memory overhead. To beat Linkerd's performance, Tachyon must adhere to its "Compile-Time Service Mesh" philosophy: if a user deploys Tachyon behind an AWS API Gateway that already handles retries, Tachyon must not compile the resiliency code.

## Objective
Integrate the highly optimized `tower` resiliency middlewares (Timeout, Retry, Limit) into the `core-host` Axum router, but gate them strictly behind a Cargo feature flag (`resiliency`). Update the `integrity.lock` schema so developers can declaratively define resiliency policies per route, which will only be enforced if the Host was compiled with the feature enabled.

## Scope
- Update `core-host/Cargo.toml` to add `tower` dependencies as `optional = true` and create a `resiliency` feature flag.
- Update `RouteConfig` in `tachyon-cli` to support `timeout_ms` and `retry_policy` configurations.
- Use conditional compilation (`#[cfg(feature = "resiliency")]`) in the Axum router setup.
- If enabled, apply `tower::timeout::TimeoutLayer` and a custom `tower::retry::RetryLayer` to the execution pipeline.

## Success Metrics
- Compiling `core-host` without the `--features resiliency` flag produces a smaller binary that contains zero Tower retry/timeout state machines, guaranteeing absolute zero overhead.
- Compiling with the feature flag enables the policies: a route configured with a 500ms timeout returns an HTTP 504 if the WASM module is too slow, and a flaky WASM module is automatically retried on 503 errors.