# Tasks: Change 050 Implementation

**Agent Instruction:** Implement the asynchronous logging pipeline. Your absolute priority is to ensure that a User FaaS calling `stdout` or `stderr` NEVER blocks the host OS threads waiting for disk or console I/O. Use 4-space indentation for code examples.

## [TASK-1] Intercept Wasmtime Output
1. In the `core-host` Wasmtime engine setup, do not allow WASI to inherit the host's standard output.
2. Use `wasi_common::pipe::WritePipe` or a custom implementation of `wasi_common::WasiFile` to capture all writes to file descriptors 1 (`stdout`) and 2 (`stderr`).

## [TASK-2] The Non-Blocking Memory Queue
1. Create a global `tokio::sync::mpsc::channel` with a fixed buffer size (e.g., 64,000 messages) to act as the Log Queue.
2. Inside your custom `WasiFile` implementation, take the captured bytes, format them into a `LogEntry` struct (adding `target_name`, `timestamp`, and `stream_type`), and push them to the Log Queue.
3. **Critical:** Use `try_send()`. If the channel is full, drop the log immediately and silently. Never use `.await` or block the WASI execution thread to wait for log queue space.

## [TASK-3] System FaaS Logger Implementation
1. Create the `system-faas-logger.wasm` component. It should expose an export function to receive an array of `LogEntry` JSON objects.
2. In the `core-host`, spawn a detached Tokio background task.
3. This task continuously `recv()` from the Log Queue, batches logs together (e.g., every 500ms or 1000 logs), and instantiates/calls `system-faas-logger.wasm` to process the batch.

## Validation Step
1. Write a dummy User FaaS containing a loop that prints 100,000 lines to `stdout`.
2. Measure the execution time of this FaaS on the host.
3. Compare it to the same FaaS running with WASI directly inheriting the host OS `stdout`. 
4. The async implementation must be significantly faster and show 0% I/O Wait in system monitoring tools.