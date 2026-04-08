# Proposal: Change 050 - Asynchronous System Logging

## Context
In WebAssembly (WASI), writes to `stdout` and `stderr` are synchronous by default. If a User FaaS generates intense log traffic, the Rust `core-host` might block on file/console I/O (I/O Wait), causing massive latency spikes for all other co-hosted FaaS. We need to decouple execution from logging.

## Objective
1. Capture all FaaS `stdout/stderr` streams at the host level using non-blocking pipes.
2. Redirect these logs to an in-memory asynchronous ring buffer.
3. Use a dedicated `system-faas-logger` to consume this buffer and export logs to external providers or persistent storage.

## Success Metrics
- A FaaS performing 100,000 log writes per second does not increase the request latency of a neighboring FaaS on the same node.
- Logs are preserved and delivered even if the User FaaS instance is destroyed immediately after execution.
