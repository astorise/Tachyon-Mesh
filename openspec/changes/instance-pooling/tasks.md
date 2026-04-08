# Tasks: Change 041 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the FaaS scaling limits and instance pool in Rust. Do not use nested code blocks in your outputs.

## [TASK-1] Update Configuration and Host State
1. Update the `integrity.lock` deserialization structs to include an optional `scale` field containing `min` and `max` integers.
2. In the `core-host`, create an `InstancePool` struct for each loaded route. This struct should contain a thread-safe collection (like `crossbeam::SegQueue` or `tokio::sync::Mutex<Vec>`) holding instantiated Wasmtime `Store` and `Instance` objects, alongside atomic counters for `current_total`.

## [TASK-2] Implement the Pre-warming Logic
1. Upon loading a new configuration, iterate over all targets.
2. For each target, read `scale.min`.
3. Loop `min` times: instantiate the Wasmtime module and push it into the `InstancePool`.

## [TASK-3] Implement the Acquisition & Release Queue
1. In the main request dispatch loop, replace the direct instantiation code with an `acquire()` method call on the route's `InstancePool`.
2. Inside `acquire()`: Try to take an idle instance. If none exist, check if `current_total < max`. If true, instantiate a new one. If false, use `tokio::sync::Notify` or an async channel to wait until an instance is returned.
3. Wrap the WASM execution in a construct (like a custom Rust `Drop` implementation or a simple `try/finally` equivalent pattern) that guarantees the instance is pushed back to the pool via a `release()` method once the HTTP response is generated.
4. If a request waits in the `acquire()` queue for more than 5 seconds, return a standard HTTP 503 Service Unavailable (or forward to `system-faas-buffer` as defined in Change 038).

## Validation Step
1. Configure a target with `scale: { min: 0, max: 2 }`.
2. Create a dummy WASM FaaS that intentionally sleeps for 2 seconds before returning a 200 OK.
3. Fire 10 concurrent curl requests at this route simultaneously.
4. Verify via host logs that exactly 2 instances are created and running concurrently.
5. Verify that the remaining 8 requests are queued and processed in batches of 2, proving the concurrency cap and release mechanics work perfectly without dropping traffic.