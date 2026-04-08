# Proposal: Change 033 - Ephemeral Volumes Garbage Collector

## Context
In Change 022, we introduced Persistent Volumes, allowing FaaS modules to read/write state. When mapped to host RAM disks (e.g., `tmpfs` or `/dev/shm`) to act as ultra-fast caches, this state becomes a severe memory leak risk. FaaS instances write files, but if they never delete them, the Host will eventually crash from memory exhaustion. We need an automated, asynchronous Garbage Collector (GC) to purge stale data.

## Objective
Introduce a configurable `ttl_seconds` (Time-To-Live) parameter to the `VolumeConfig` schema. 
Implement a low-priority background worker in the `core-host` that periodically sweeps these specific directories. If a file or subdirectory has not been modified for longer than its volume's TTL, the GC safely deletes it.

## Scope
- Update `tachyon-cli` to accept a `--ttl` flag when defining volumes.
- Update `VolumeConfig` in `core-host` to parse `ttl_seconds`.
- Create a `VolumeSweeper` Tokio task that wakes up periodically (e.g., every 60 seconds).
- The sweeper iterates through all configured volumes that have a TTL, checks `std::fs::metadata` for the `modified` timestamp, and removes stale files using `std::fs::remove_file` or `remove_dir_all`.

## Success Metrics
- A volume configured with `ttl_seconds: 300` (5 minutes) automatically loses files that haven't been touched in 5 minutes.
- The GC runs entirely in the background without causing latency spikes in the Axum HTTP router.
- The host is protected from Out-Of-Memory (OOM) crashes when serving high-throughput caching FaaS modules.