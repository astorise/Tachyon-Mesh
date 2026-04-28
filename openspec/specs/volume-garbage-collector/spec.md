# volume-garbage-collector Specification

## Purpose
TBD - created by archiving change volume-garbage-collector. Update Purpose after archive.
## Requirements
### Requirement: Volume mounts can define a TTL for garbage collection
The integrity manifest SHALL allow volume mount definitions to declare `ttl_seconds` so ephemeral data can be evicted automatically after it becomes stale.

#### Scenario: A volume mount declares a TTL
- **WHEN** a target mount specifies `ttl_seconds`
- **THEN** the host loads that TTL into the runtime configuration without breaking manifests that omit it

### Requirement: The host runs a periodic background sweeper for TTL-managed paths
The host SHALL run a periodic background task that collects unique TTL-managed host paths and delegates filesystem scanning to blocking worker threads.

#### Scenario: The sweeper tick runs
- **WHEN** the periodic garbage collector wakes up
- **THEN** it gathers the configured host paths with TTL settings
- **AND** scans them through blocking filesystem work so the async executor is not stalled

### Requirement: Stale files are removed based on modified time
The garbage collector SHALL delete files or directories whose age exceeds the configured TTL and SHALL tolerate races where files disappear or remain in active use.

#### Scenario: A file is older than the configured TTL
- **WHEN** the sweeper examines an entry whose last modified time is older than `ttl_seconds`
- **THEN** it deletes that entry and records the deletion in host logs

#### Scenario: A file cannot be removed because of a race
- **WHEN** the sweeper encounters a `NotFound` or `PermissionDenied` style error while deleting a stale entry
- **THEN** it handles the error gracefully without crashing the host

### Requirement: GC sweeper tolerates filesystem races
The `system-faas-gc` `sweep_directory` logic SHALL handle filesystem race conditions (`PermissionDenied`, `NotFound`, locked entries) by logging the error and continuing the sweep, rather than propagating the error and trapping the Wasm execution.

#### Scenario: A concurrent file deletion does not abort the sweep
- **WHEN** the GC sweeper attempts to remove a stale file
- **AND** another process deletes or locks the file between the listing and the removal call
- **THEN** the sweeper catches the resulting `NotFound` or `PermissionDenied` error
- **AND** logs a warning identifying the entry
- **AND** continues processing the remaining entries in the directory without trapping

### Requirement: Empty directories are removed during recursive sweep
After processing the contents of a directory recursively, the GC sweeper SHALL check whether the directory is empty and, if so, remove it via `fs::remove_dir`, with the same error tolerance applied.

#### Scenario: Nested empty directories are reaped
- **WHEN** the recursive sweep removes the last stale file in a deeply nested directory tree
- **THEN** each empty parent directory is removed in turn as the recursion unwinds
- **AND** any directory that fails to be removed (for example because a new child appeared concurrently) is logged and skipped
- **AND** the host's inode usage decreases monotonically as ghost directories are cleared

