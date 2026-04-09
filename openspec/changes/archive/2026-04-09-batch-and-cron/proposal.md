# Proposal: Change 034 - Run-to-Completion (Batch FaaS) & System Cron

## Context
Our current architecture strictly assumes a "Reactor" execution model where WASM modules are event-driven (HTTP, gRPC, WebSockets). However, a complete Cloud-Native platform must support background tasks, data pipelines, and scheduled maintenance (like the Volume Garbage Collector). We need to support the WASI "Command" model for Batch processing, allowing seamless integration with Kubernetes `Job` and `CronJob` resources.

## Objective
1. Refactor the Volume GC (Change 033) into a standalone `system-faas-gc.wasm` module.
2. Introduce a new execution mode in the `core-host` CLI: `tachyon-host run <module.wasm>`, which bypasses the Axum web server entirely, executes the FaaS `main` function, and exits the process with the FaaS's exit code.
3. Update `integrity.lock` to support a `cron` trigger for System FaaS within the persistent Mesh (for users not relying on external K8s CronJobs).

## Scope
- Update `tachyon-cli` to distinguish between `type: "http"` and `type: "batch"`.
- Implement a one-shot execution path in `core-host` that uses Wasmtime's typed `Command` interface (calling the exported `wasi:cli/run` function).
- Prove the architecture by implementing the Volume Garbage Collector purely in WebAssembly, mapping the host's `/dev/shm` to the guest and running it as a scheduled batch.

## Success Metrics
- A `batch` FaaS can be executed via CLI. If it panics or returns an error, the `core-host` process exits with code 1 (failing the K8s Job). If it succeeds, it exits with code 0 (completing the K8s Job).
- The Volume GC logic is completely removed from the Rust Host and successfully operates as a WASM module using standard `wasi::filesystem` capabilities.