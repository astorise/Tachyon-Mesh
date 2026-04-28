## ADDED Requirements

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
