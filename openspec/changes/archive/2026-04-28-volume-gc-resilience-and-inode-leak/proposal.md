# Proposal: GC Resilience and Inode Leak Fix

## Context
The current implementation of the `system-faas-gc` module contains two critical flaws that threaten the long-term stability of the `core-host` and the underlying operating system:
1. **Spec Violation (Race Conditions):** The existing `volume-garbage-collector/spec.md` strictly mandates that the sweeper must tolerate race conditions (e.g., a file being deleted or locked by another process). However, the current Rust code uses the `?` operator for file deletion (`fs::remove_file(&entry_path)?`). If a single file throws a `PermissionDenied` or `NotFound` error, the entire Wasm execution traps and crashes, halting the GC process.
2. **Silent Inode Exhaustion:** The recursive `sweep_directory` function successfully deletes stale files but never removes the parent directories once they become empty. Over time, highly dynamic workloads will generate millions of empty "ghost" directories, eventually exhausting the host filesystem's Inode table and causing a catastrophic system-wide failure.

## Proposed Solution
We will refactor the `sweep_directory` logic in `systems/system-faas-gc/src/main.rs`:
- Implement a `match` block around filesystem operations to catch and log errors (graceful degradation) instead of panicking.
- Introduce a post-sweep check during the recursive directory traversal. If a directory is found to be completely empty after its stale contents are removed, the directory itself will be deleted using `fs::remove_dir`.

## Objectives
- Ensure the GC module never crashes due to expected filesystem races.
- Prevent OS-level Inode exhaustion by cleaning up hierarchical debris.