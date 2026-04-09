# Tasks: Change 026 Implementation

- [x] Add `arc-swap` and move the live runtime configuration behind an atomically swappable shared state.
- [x] Route all HTTP requests through an Axum fallback handler that resolves sealed paths from the current runtime state.
- [x] Implement `SIGHUP` hot reloads that rebuild the runtime state from `integrity.lock` and keep the previous state on failure.
- [x] Implement graceful shutdown for `SIGTERM` / `SIGINT`, including background worker teardown.
- [x] Cover hot reload and graceful shutdown with automated tests and re-run `openspec validate --all`.
