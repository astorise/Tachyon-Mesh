# Implementation Tasks

## Phase 1: Temporary File Logic
- [x] Open `systems/system-faas-model-broker/src/lib.rs`.
- [x] Locate the logic responsible for creating and writing the incoming model stream to disk.
- [x] Modify the file creation logic to append `.part` to the requested filename. (Already in place via `staging_path()`.)

## Phase 2: Atomic Completion
- [x] Ensure `flush()` is called after each chunk write so data is durably appended (already present).
- [x] Implement the `fs::rename` call to atomically move the `.part` file to its final filename (already in place in `commit_upload`).

## Phase 3: Immediate Failure Cleanup
- [x] Add `cleanup_staging(upload_id)` helper that best-effort-removes the `.part` and metadata file.
- [x] Call `cleanup_staging` from `commit_upload` on hash mismatch so a corrupted upload doesn't leak its staging file.
- [x] Add a `POST/DELETE /admin/models/abort/{upload_id}` endpoint so the orchestrator can free a slot when a peer disconnects mid-upload (the broker is a request-driven Wasm guest and cannot observe disconnects in-band).
- [x] Hard-crash orphans are reaped by `system-faas-gc`'s TTL sweep — the documented fallback in the proposal.

## Phase 4: Validation
- [x] Unit test `abort_extracts_upload_id` — confirms abort path parsing.
- [x] Unit test `cleanup_staging_is_idempotent_on_missing_files` — confirms the hash-mismatch path is safe even when the `.part` is already gone (e.g. concurrent gc).
