# Tasks: Change 051 Implementation

**Agent Instruction:** Implement the Graceful Draining state machine. This touches the core HTTP router and the Garbage Collector. Do not drop active HTTP connections during a Hot-Reload. Use 4-space indentation for code examples.

## [TASK-1] Instance Reference Counting
1. In the `InstancePool` (Change 041) of the `core-host`, add an `AtomicUsize` field named `active_requests`.
2. Increment this counter immediately when an HTTP request is assigned to a FaaS target.
3. Decrement this counter exactly when the HTTP response has been fully flushed to the client (use a `Drop` guard or a reliable `finally` equivalent).

## [TASK-2] Dual-State Routing
1. Update the Hot-Reload configuration listener (Change 026).
2. When a new `integrity.lock` is detected, do not immediately clear the existing routing table.
3. Flag the old target definitions as `State::Draining` and insert the new target definitions as `State::Active`.
4. Update the HTTP dispatcher to ONLY route new incoming HTTP requests to targets marked as `State::Active`.

## [TASK-3] The Reaper (Garbage Collector)
1. Spawn a background ticker task (e.g., running every 1 second).
2. The Reaper iterates over all `Draining` targets in memory.
3. If a target's `active_requests` counter is exactly `0`, safely destroy its `InstancePool`, unmount its volumes, and remove it from memory.
4. Implement a "Kill Switch": If a target has been in the `Draining` state for more than 30 seconds, forcibly terminate it and drop its instances, even if `active_requests > 0` (this prevents memory leaks from hung user code).

## Validation Step
1. Deploy a `v1` User FaaS that artificially sleeps for 10 seconds before returning "Hello V1".
2. Send an HTTP request to this FaaS.
3. At second 2, replace the FaaS with `v2` (returns "Hello V2") and trigger a Hot-Reload.
4. Verify that the initial HTTP request successfully completes at second 10 and returns "Hello V1".
5. Send a new HTTP request at second 3; verify it instantly returns "Hello V2".
6. Check host logs to confirm that `v1` is destroyed by the Reaper only after second 10.