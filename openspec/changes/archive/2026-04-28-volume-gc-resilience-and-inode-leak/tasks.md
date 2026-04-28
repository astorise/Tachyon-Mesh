# Implementation Tasks

## Phase 1: Race tolerance
- [x] Open `systems/system-faas-gc/src/main.rs`.
- [x] Replace each `?`-style filesystem op (`read_dir`, entry iteration, `metadata`, `modified`, `remove_file`) with a `match` block that logs the error and continues the sweep.
- [x] Surface a `SweepStats { removed_files, removed_dirs }` return value so callers can observe progress without parsing logs.

## Phase 2: Empty-directory cleanup
- [x] After processing a directory's contents, check whether it has become empty via `fs::read_dir(...).next()`.
- [x] If empty, remove it via `fs::remove_dir`. Log + continue on failure (a child may have appeared concurrently).

## Phase 3: Validation
- [x] Unit test `sweep_removes_stale_files` — assert stale files plus the now-empty parent are reaped.
- [x] Unit test `sweep_tolerates_race_on_missing_file` — pre-delete an entry; assert the sweep finishes without panicking.
- [x] Unit test `sweep_reaps_nested_empty_dirs` — assert the recursion unwinds and removes each empty parent.
