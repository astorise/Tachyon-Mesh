# Implementation Tasks

## Phase 1: Watcher Integration
- [ ] Add `notify = "6.1"` and `futures-util` to `core-host/Cargo.toml`.
- [ ] In `core-host/src/main.rs`, implement the `spawn_watcher` function.
- [ ] Ensure the watcher correctly handles file moves (many editors save by moving a temp file over the original).

## Phase 2: Refactoring Bootstrap Logic
- [ ] Refactor the current bootstrap code into a reusable function `load_runtime(path: &Path) -> Result<IntegrityRuntime>`.
- [ ] Update the background task to call this function and perform the `state.runtime.store()` call upon success.

## Phase 3: Middleware Verification
- [ ] Verify in `server_h3.rs` that the runtime is loaded *per request* using `state.runtime.load()` and not cached in the listener loop.

## Phase 4: Validation
- [ ] **Test Zero-Downtime:** Start a large download (simulated) via an HTTP/3 route. While downloading, change a resource alias or a FaaS dependency in `integrity.lock`.
- [ ] Verify the download finishes successfully (using the old config) while new requests immediately see the updated configuration.
- [ ] **Test Robustness:** Intentionally write a corrupted JSON to `integrity.lock` and verify the `core-host` remains alive and continues serving with the last known good configuration.