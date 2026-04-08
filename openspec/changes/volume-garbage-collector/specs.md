# Specifications: Volume GC Architecture

## 1. Schema Update v9 (`integrity.lock`)
Update the `VolumeConfig` introduced in Change 022.

    {
        "targets": [
            {
                "name": "cache-api",
                "module": "guest-cache.wasm",
                "volumes": [
                    {
                        "host_path": "/dev/shm/tachyon_cache",
                        "guest_path": "/data",
                        "readonly": false,
                        "ttl_seconds": 3600 // 1 hour TTL
                    }
                ]
            }
        ]
    }

## 2. Background Sweeper Logic (`core-host`)
The Host MUST spawn a dedicated Tokio task at startup:
- Create an interval `tokio::time::interval(Duration::from_secs(60))`.
- On each tick, obtain a read lock on the `TargetRegistry` (or the `AppState` from Change 026).
- Extract a unique list of all `host_path` directories that have a `ttl_seconds` > 0.
- For each directory, spawn a blocking task (`tokio::task::spawn_blocking`) to perform the file system I/O, preventing the async executor from stalling.

## 3. Eviction Heuristic (Modified Time)
Inside the blocking task:
- Use `std::fs::read_dir` to iterate over the contents of `host_path`.
- For each entry, read `entry.metadata()?.modified()?`.
- Calculate `SystemTime::now().duration_since(modified_time)`.
- If the duration exceeds `ttl_seconds`, delete it.
- *Note: To avoid deleting directories while a FaaS is currently writing to them, handle `std::io::ErrorKind::PermissionDenied` or `NotFound` gracefully.*