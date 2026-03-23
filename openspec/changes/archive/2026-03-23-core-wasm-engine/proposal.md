## Why

The current Knative and Linkerd based FaaS stack carries enough sidecar and control-plane overhead to make medium-sized deployments inefficient. We need a lean execution path now so the platform can prove out a single-process Rust host that runs WASI guest modules directly.

## What Changes

- Initialize a Rust workspace containing the `core-host` runtime and the `guest-example` guest module.
- Add a new WASM execution capability that defines how the host loads, links, and invokes a compiled `wasm32-wasip1` guest.
- Establish a minimal verification flow that builds the guest module and confirms guest output is emitted through inherited stdio.
- Explicitly defer HTTP routing, signature validation, and observability concerns to later changes.

## Capabilities

### New Capabilities

- `wasm-function-execution`: Execute a compiled WASI guest module from the Rust host through a stable exported entrypoint.

### Modified Capabilities

## Impact

- Affects the Rust workspace layout and build pipeline.
- Introduces `wasmtime`, `wasmtime-wasi`, `tokio`, and `anyhow` as foundational runtime dependencies.
- Establishes the first spec contract for host-to-guest execution behavior.
