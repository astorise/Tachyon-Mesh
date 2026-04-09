# Tasks: Change 051 Implementation

**Agent Instruction:** Implement the Graceful Draining state machine. This touches the core HTTP router and the Garbage Collector. Do not drop active HTTP connections during a Hot-Reload. Use 4-space indentation for code examples.

## [TASK-1] Instance Reference Counting
- [x] In the `InstancePool` (Change 041) of the `core-host`, add an `AtomicUsize` field named `active_requests`.
- [x] Increment this counter immediately when an HTTP request is assigned to a FaaS target.
- [x] Decrement this counter exactly when the HTTP response has been fully flushed to the client (use a `Drop` guard or a reliable `finally` equivalent).

## [TASK-2] Dual-State Routing
- [x] Update the Hot-Reload configuration listener (Change 026).
- [x] When a new `integrity.lock` is detected, do not immediately clear the existing routing table.
- [x] Flag the old target definitions as `State::Draining` and insert the new target definitions as `State::Active`.
- [x] Update the HTTP dispatcher to ONLY route new incoming HTTP requests to targets marked as `State::Active`.

## [TASK-3] The Reaper (Garbage Collector)
- [x] Spawn a background ticker task (e.g., running every 1 second).
- [x] The Reaper iterates over all `Draining` targets in memory.
- [x] If a target's `active_requests` counter is exactly `0`, safely destroy its `InstancePool`, unmount its volumes, and remove it from memory.
- [x] Implement a "Kill Switch": If a target has been in the `Draining` state for more than 30 seconds, forcibly terminate it and drop its instances, even if `active_requests > 0` (this prevents memory leaks from hung user code).

## Validation Step
- [x] Deploy a `v1` User FaaS that artificially sleeps for 10 seconds before returning "Hello V1".
- [x] Send an HTTP request to this FaaS.
- [x] At second 2, replace the FaaS with `v2` (returns "Hello V2") and trigger a Hot-Reload.
- [x] Verify that the initial HTTP request successfully completes at second 10 and returns "Hello V1".
- [x] Send a new HTTP request at second 3; verify it instantly returns "Hello V2".
- [x] Check host logs to confirm that `v1` is destroyed by the Reaper only after second 10.
