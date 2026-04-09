## ADDED Requirements

### Requirement: Host hot-reloads sealed routes without restarting the HTTP listener
The `core-host` runtime SHALL keep its active route registry, Wasmtime engine, and per-route concurrency controls behind an atomically swappable shared state so a `SIGHUP` can reload `integrity.lock` without rebinding the Axum listener.

#### Scenario: A new sealed route becomes active after `SIGHUP`
- **WHEN** the running host receives `SIGHUP`
- **AND** the updated `integrity.lock` passes signature verification and runtime validation
- **THEN** the host rebuilds its runtime state from disk
- **AND** newly sealed routes become reachable on the existing TCP listener
- **AND** in-flight requests continue using the previous runtime state until they complete

#### Scenario: A broken manifest does not replace the active state
- **WHEN** the running host receives `SIGHUP`
- **AND** the updated `integrity.lock` fails signature verification, route validation, or runtime construction
- **THEN** the host logs the reload failure
- **AND** the previously active runtime state remains in service

### Requirement: Host drains requests before exiting on shutdown signals
The `core-host` runtime SHALL stop accepting new connections on `SIGTERM` or `SIGINT` and allow in-flight request execution to finish before the process exits.

#### Scenario: Shutdown waits for an executing request
- **WHEN** a request is already executing when the host receives `SIGTERM`
- **THEN** the HTTP listener begins graceful shutdown
- **AND** the in-flight request is allowed to finish and return its response
- **AND** the host exits only after the request has completed
