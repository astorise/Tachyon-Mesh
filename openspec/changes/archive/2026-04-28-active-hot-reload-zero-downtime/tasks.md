# Implementation Tasks

## Phase 1: Watcher Integration
- [x] Add `notify = "6.1"` to `core-host/Cargo.toml` (no `futures-util` needed — the watcher pumps via `tokio::sync::mpsc`).
- [x] Implement `spawn_manifest_file_watcher` in `core-host/src/main.rs`. It watches the manifest's parent directory non-recursively and filters events by filename so atomic-rename-style saves still match.
- [x] Coalesce a flurry of OS events behind a 250 ms debounce window so multi-step saves trigger a single reload.

## Phase 2: Reuse the existing reload pipeline
- [x] The watcher calls the existing `reload_runtime_from_disk(&state)`. That function already builds the new runtime, marks the previous runtime as draining, performs the `state.runtime.store()` atomic swap, and reaps drained generations. No additional plumbing is needed.
- [x] On validation failure, `reload_runtime_from_disk` returns an error; the watcher logs it and keeps the previous runtime active.

## Phase 3: Middleware verification
- [x] HTTP / HTTP-3 / mTLS handlers all call `state.runtime.load()` per-request — confirmed by inspection (`server_h3.rs` and the axum handlers go through `state.runtime`). No listener-cached runtime references.

## Phase 4: Validation
- [x] Existing tests `reload_runtime_from_disk_swaps_in_new_routes`, `reload_runtime_from_disk_keeps_previous_state_on_invalid_manifest`, and `reload_runtime_from_disk_drains_previous_generation_until_response_flush` cover the swap semantics.
- [ ] (Manual) End-to-end: hold a long-running HTTP/3 stream open while editing `integrity.lock` and confirm the in-flight request sees the old runtime through completion while a new request immediately picks up the new one. Left for the homelab smoke test.
