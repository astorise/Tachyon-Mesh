# Specifications: Batch & Command Architecture

## 1. Execution Models (Reactor vs Command)
Wasmtime and WASI Preview 2 strictly differentiate between components:
- Reactor: Exports functions like handle-http or on-connect. Does not have a main() function. The host keeps the instance alive.
- Command: Exports wasi:cli/run#run. The host instantiates it, calls run(), and drops the instance immediately when it returns.

## 2. CLI and K8s Job Integration
To allow Kubernetes to manage Tachyon batches, the core-host binary must accept a new subcommand. Example usage:

    $ core-host run --manifest integrity.lock --target my-batch-job

When invoked:
1. The host skips starting the Tokio TCP/UDP listeners.
2. It parses the manifest, sets up the WasiCtx (injecting ENV vars and directory mounts).
3. It instantiates the target WASM component.
4. It calls the run() export.
5. It captures the Result and maps it to std::process::exit(0) or 1.

This allows a Kubernetes YAML to easily wrap the Tachyon host binary using standard container arguments (command: ["core-host", "run", "--target", "daily-report"]).

## 3. The GC System FaaS (system-faas-gc)
The Garbage Collector is compiled as a Command component.
- The host maps the volume (e.g., /dev/shm/tachyon maps to /cache inside the guest).
- The host passes TTL_SECONDS=300 as an environment variable.
- The WASM module uses standard Rust std::fs::read_dir("/cache"), checks metadata, and deletes old files.
- It returns Ok(()) and exits.