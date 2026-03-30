# Proposal: Wasmtime Pooling And Per-Route Concurrency

## Why

`core-host` currently creates guest instances on demand for every request and does not enforce a
route-scoped concurrency budget. Under burst traffic, the host can spend unnecessary time churning
through Wasmtime allocations while allowing a single route to consume disproportionate execution
capacity.

## What Changes

- Extend the sealed route schema in `integrity.lock` with `min_instances` and
  `max_concurrency`.
- Teach `tachyon-cli generate` to accept per-route scaling overrides without breaking the existing
  `--route`, `--system-route`, and `--secret-route` flows.
- Configure Wasmtime with the pooling allocator and size the pool from the sealed runtime
  configuration plus the existing guest memory ceiling.
- Enforce `max_concurrency` with `tokio::sync::Semaphore` in the Axum handler and return HTTP 503
  when a request cannot obtain a permit within five seconds.

## Non-Goals

- Introduce an application-managed object pool for pre-instantiated guest stores.
- Change guest module resolution away from the current path-derived contract.
- Redesign the integrity signature or CI workflow beyond the manifest and runtime behavior needed
  for this change.

## Impact

- Keeps the sealed manifest backward compatible by defaulting new route fields when older manifests
  omit them.
- Makes per-route concurrency limits explicit and enforceable by the host.
- Reduces allocator churn by switching the shared Wasmtime engine to pooled instance allocation.
