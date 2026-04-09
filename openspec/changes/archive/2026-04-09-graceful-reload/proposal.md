# Proposal: Change 026 - Graceful Shutdown & Hot Reloading

## Context
Deploying a new FaaS version (or updating the SemVer dependency graph from Change 025) currently requires stopping the `core-host` process and restarting it. This drops active HTTP connections and disrupts the internal IPC mesh. A production-grade Service Mesh must dynamically reload its configuration and WebAssembly modules without dropping a single packet, and must gracefully drain existing connections when legitimately shutting down.

## Objective
Implement a Zero-Downtime architecture in the `core-host`. 
1. **Hot Reload:** Listen for a specific OS signal (e.g., `SIGHUP`). When received, parse the `integrity.lock` again, compile any new `.wasm` files, and atomically swap the routing table using `arc-swap`, all without stopping the Axum TCP listener.
2. **Graceful Shutdown:** Listen for `SIGTERM`/`SIGINT`. Stop accepting new connections, but wait for currently executing Wasmtime instances to finish before exiting the process.

## Scope
- Refactor the Axum router to use a wildcard/fallback route so dynamic path resolution happens inside the handler against an `ArcSwap<TargetRegistry>` rather than statically defined Axum paths.
- Add signal handling via `tokio::signal`.
- Add the `arc-swap` dependency for lock-free configuration swapping.
- Ensure Wasmtime memory is correctly garbage-collected when an old routing table is dropped.

## Success Metrics
- Sending a `SIGHUP` (`kill -HUP <pid>`) to the host triggers a successful reload of `integrity.lock`. A new route added to the JSON becomes instantly accessible.
- Running a 10-second FaaS execution and sending `SIGTERM` (`kill -TERM <pid>`) during second 2 allows the FaaS to finish and return its HTTP 200 response before the host process actually exits.