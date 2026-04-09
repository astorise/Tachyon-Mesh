# Tasks: Change 050 Implementation

**Agent Instruction:** Implement the asynchronous logging pipeline. Your absolute priority is to ensure that a User FaaS calling `stdout` or `stderr` NEVER blocks the host OS threads waiting for disk or console I/O. Use 4-space indentation for code examples.

## [TASK-1] Intercept Wasmtime Output
- [x] In the `core-host` Wasmtime engine setup, do not allow WASI to inherit the host's standard output.
- [x] Use `wasi_common::pipe::WritePipe` or a custom implementation of `wasi_common::WasiFile` to capture all writes to file descriptors 1 (`stdout`) and 2 (`stderr`).

## [TASK-2] The Non-Blocking Memory Queue
- [x] Create a global `tokio::sync::mpsc::channel` with a fixed buffer size (e.g., 64,000 messages) to act as the Log Queue.
- [x] Inside your custom `WasiFile` implementation, take the captured bytes, format them into a `LogEntry` struct (adding `target_name`, `timestamp`, and `stream_type`), and push them to the Log Queue.
- [x] **Critical:** Use `try_send()`. If the channel is full, drop the log immediately and silently. Never use `.await` or block the WASI execution thread to wait for log queue space.

## [TASK-3] System FaaS Logger Implementation
- [x] Create the `system-faas-logger.wasm` component. It should expose an export function to receive an array of `LogEntry` JSON objects.
- [x] In the `core-host`, spawn a detached Tokio background task.
- [x] This task continuously `recv()` from the Log Queue, batches logs together (e.g., every 500ms or 1000 logs), and instantiates/calls `system-faas-logger.wasm` to process the batch.

## Validation Step
- [x] Write a dummy User FaaS containing a loop that prints 100,000 lines to `stdout`.
- [x] Measure the execution time of this FaaS on the host.
- [x] Compare it to the same FaaS running with WASI directly inheriting the host OS `stdout`.
- [x] The async implementation must be significantly faster and show 0% I/O Wait in system monitoring tools.
