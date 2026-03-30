## Summary

The change extends the existing sealed route model instead of introducing a new standalone route
schema. `tachyon-cli` continues to accept route declarations through repeatable flags, while a new
optional scaling override flag applies `min_instances` and `max_concurrency` to a named route
before the manifest is signed.

`core-host` reads those values from the embedded configuration, validates them, and uses them in
two places:

- Wasmtime is configured with the pooling allocator so instance metadata and linear memories are
  reserved from a shared pool instead of the default on-demand allocator.
- The Axum handler guards each sealed route with a dedicated `tokio::sync::Semaphore`, enforcing
  the configured `max_concurrency` budget with a five-second wait timeout.

## CLI Surface

- Keep `--route`, `--system-route`, and `--secret-route` unchanged.
- Add `--route-scale /path=min:max` as an optional repeatable override.
- Default omitted scaling values to `min_instances = 0` and `max_concurrency = 100`.
- Reject scaling overrides for unknown routes and any override with `max_concurrency = 0`.

## Runtime Behavior

- Build the semaphore map once at startup from the normalized sealed routes.
- Acquire the route semaphore before guest execution begins.
- Hold the permit for the full request handling scope so queued requests measure real end-to-end
  capacity, not just guest instantiation time.
- Return HTTP 503 when the semaphore wait exceeds five seconds.

## Pooling Strategy

- Keep the existing fuel metering and component-model support enabled.
- Add Wasmtime pooling allocation on the shared engine.
- Size the pool from the declared route concurrency while constraining memory reservations with the
  existing `guest_memory_limit_bytes` ceiling.
- Keep `min_instances` in the sealed schema for compatibility and future warm-up work, without
  introducing an application-level pre-instantiation cache in this change.

## Verification

- Validate the repaired change with OpenSpec deltas under the impacted capabilities.
- Add or update Rust tests for route scaling parsing, integrity validation defaults, and HTTP 503
  behavior when the route concurrency budget is exhausted.
- Regenerate `integrity.lock`, then run formatting, linting, tests, and release builds before
  pushing to GitHub Actions.
