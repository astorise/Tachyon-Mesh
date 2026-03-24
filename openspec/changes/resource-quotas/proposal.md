# Proposal: Change 003 - Resource Quotas & Isolation

## Context
By moving away from Kubernetes Pods and Knative, we lose OS-level namespace and cgroup isolation. If a WASM guest function contains an infinite loop or attempts to allocate gigabytes of RAM, it could crash the single Rust Host process, causing a "Noisy Neighbor" cascading failure for all other running functions.

## Objective
Leverage `wasmtime`'s internal sandboxing capabilities to enforce strict, deterministic quotas on every guest execution. We will limit the maximum memory a WASM instance can allocate and implement a "Fuel" system (instruction counting) to prevent infinite CPU loops.

## Scope
- Configure `wasmtime::Config` to enable fuel consumption.
- Enforce a strict memory limit (e.g., 50 MB) per WASM instance using a `ResourceLimiter`.
- Inject a fixed amount of Fuel (instructions) into the `Store` before execution.
- Catch `wasmtime::Trap` errors (Out of Fuel, Out of Memory) and translate them into graceful HTTP 500 responses without crashing the Axum server.
- Create a malicious guest function to prove the isolation works.

## Out of Scope
- Dynamic quota allocation based on user configuration (hardcoded limits are fine for this iteration).
- Network isolation (WASI already prevents arbitrary socket creation by default).

## Success Metrics
- Executing a normal FaaS function succeeds and returns HTTP 200.
- Executing a FaaS function with an infinite loop is trapped by the Host within milliseconds, returning an HTTP 500 error.
- Executing a FaaS function that tries to allocate 100MB of RAM is trapped, returning an HTTP 500 error.
- The Rust Host process NEVER panics or crashes during these malicious executions.