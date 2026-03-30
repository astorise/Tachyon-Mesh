# Proposal: Change 022 - Persistent Volumes & WASI File System

## Context
While FaaS execution is stateless by design, many enterprise workloads require reading large datasets, caching files, or writing persistent output (e.g., reports, SQLite databases). WebAssembly operates in a strict sandbox with zero access to the host file system. We need a secure, declarative way to mount host directories into the WASM guest, similar to Docker volumes.

## Objective
Implement WASI "Preopened Directories" in the `core-host`. We will update the `integrity.lock` configuration schema to allow developers to declare volume mappings (`host_path` to `guest_path`) with read/write permissions. The host will intercept these declarations and securely wire them into the `WasiCtx` before instantiating the module.

## Scope
- Update `tachyon-cli` to support defining volume mappings per route.
- Update the `RouteConfig` struct in `core-host` to parse these volumes.
- Modify the `faas_handler` to iterate over the configured volumes and inject them using `WasiCtxBuilder::preopened_dir`.
- Create a test FaaS (`guest-volume`) that writes a string to a file in a mounted volume and reads it back to prove persistence.

## Success Metrics
- A guest FaaS can successfully read and write to its designated `guest_path` (e.g., `/app/data`).
- A guest FaaS attempting to access a path outside its designated preopened directory (e.g., `/etc` or `../`) receives a strict `PermissionDenied` error from the WASI runtime.
- The state persists on the host's actual file system after the FaaS instance is destroyed.