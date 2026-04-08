# Proposal: Change 040 - Volume Broker & Concurrency Control

## Context
Standard POSIX filesystems and the `wasi:filesystem` interface do not natively resolve massive concurrent write operations without relying on OS-level locks. If 10,000 FaaS instances attempt to write to a shared mounted volume simultaneously, the underlying Linux kernel will either corrupt the data (race conditions) or block the host's executor threads waiting for I/O locks, destroying Tachyon's nanosecond latency guarantees. Adding a complex Lock Manager directly into the Rust `core-host` violates our microkernel architecture.

## Objective
Implement a Privilege Separation pattern for volume I/O. 
1. The `core-host` will strictly enforce `Read-Only` (RO) volume mounts for all User FaaS. 
2. A specialized System FaaS, the `system-faas-storage-broker`, will be instantiated as a Singleton per volume, holding the exclusive `Read/Write` (RW) permission. 
3. User FaaS needing to mutate files will send ultra-fast internal IPC requests (via the Mesh) to the Storage Broker, which will queue and execute the writes sequentially, guaranteeing data integrity without blocking the host.

## Scope
- Update `integrity.lock` parsing to validate volume permissions (`ro` vs `rw`) based on FaaS roles.
- Build `system-faas-storage-broker.wasm` to act as a centralized, non-blocking I/O queue.
- Provide a lightweight SDK/wrapper for User FaaS so that `fs.writeFile()` calls are seamlessly intercepted and translated into Mesh HTTP requests to the broker.

## Success Metrics
- 10,000 concurrent FaaS instances can request writes to the exact same file simultaneously.
- Zero data corruption occurs.
- The `core-host` executor threads experience zero I/O blocking wait time.
- The final file reflects all 10,000 sequential writes perfectly.