# Design: Admission Control Logic

## 1. Resource Manifest Extension
The `capabilities.json` schema in Tachyon-UI must be updated to support:
- `resource_policy.min_ram_gb`: A strict threshold required for execution.
- `resource_policy.admission_strategy`: Either `fail_fast` (strict rejection) or `mesh_retry` (offloading).

## 2. Decision Matrix
The `core-host/src/auth.rs` component (or the main router) will execute the following logic:
- **Synchronous Request**:
    - Local RAM OK? -> Instantiate and compute.
    - Else, Mesh Neighbor OK? -> Transparent HTTP/3 proxying to the neighbor.
    - Else -> Return HTTP `503 Service Unavailable` with `X-Tachyon-Reason: Insufficient-Cluster-Resources`.
- **Asynchronous Request**:
    - If `system_load > 85%`, accept the job into the message broker but immediately emit a `tachyon.notify.pressure` event via WebSocket to inform the client of potential delays.

## 3. UI Integration
The Tachyon-Studio interface must listen to the telemetry stream. If an async task is submitted while the node is nearing saturation, an alert toast or warning icon must appear on the Message Broker view to warn the user about deferred execution.