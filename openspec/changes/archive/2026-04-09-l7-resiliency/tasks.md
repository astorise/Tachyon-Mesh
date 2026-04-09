# Tasks: Change 028 Implementation

**Agent Instruction:** Read the `proposal.md` and `specs.md`. Implement the timeouts and retries strictly behind the `resiliency` feature flag to ensure zero overhead when disabled.

## [TASK-1] Configure Cargo and CLI
- [x] In `core-host/Cargo.toml`, add the `resiliency` feature and the optional `tower` dependency.
- [x] In `tachyon-cli`, create the `ResiliencyConfig` and `RetryPolicy` structs. Add them as an `Option<ResiliencyConfig>` to the `Target` struct.

## [TASK-2] Implement the Feature-Gated Layers
- [x] In `core-host/src`, create a new module `resiliency.rs`.
- [x] Wrap the entire content of this module in `#[cfg(feature = "resiliency")]`.
- [x] Inside, implement your custom `tower::retry::Policy` that checks the response status against the config's `retry_on` list.
- [x] Expose a helper function that takes the base Axum `Router` (or `Service`) and wraps it in the `TimeoutLayer` and `RetryLayer` based on the target's configuration.

## [TASK-3] Wire the Router Conditionally
- [x] In `core-host/src/main.rs`, during the setup of your fallback route or dynamic execution handler, call the resiliency wrapper.
- [x] Ensure you provide a fallback empty implementation using `#[cfg(not(feature = "resiliency"))]` that simply passes the request through without wrapping it in any Tower layers.

## Validation Step
- [x] Create a `guest-flaky.wasm` that returns an HTTP `503` 70% of the time, and `200` otherwise. Config it with 5 retries and a 500ms timeout.
- [x] **Test Overhead-Free:** Run `cargo run -p core-host --release`. Trigger the route. It should fail instantly with a `503` (no retries occur).
- [x] **Test Resiliency:** Run `cargo run -p core-host --release --features resiliency`. Trigger the route. The host should retry internally and ultimately return a `200 OK`.
- [x] Modify the guest to sleep for 2 seconds. Trigger it. The host should return `504 Gateway Timeout` after 500ms.
