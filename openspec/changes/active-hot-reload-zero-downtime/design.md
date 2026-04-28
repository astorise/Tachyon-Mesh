# Design: Watcher and Lifecycle Management

## 1. File Watcher Implementation (`core-host/src/main.rs`)
In the `main` function, after the initial bootstrap, spawn a dedicated background task.

### Logic:
- Use the `notify` crate to watch the path specified by `--lockfile`.
- Implement a small debouncing delay (e.g., 500ms) to avoid multiple reloads during a multi-stage file write.

## 2. The Swap Procedure
When the watcher triggers:
1. **Validation:** Perform a fresh `bootstrap_integrity` on the new file. If parsing fails, log the error and **ABORT** (do not crash the host).
2. **Instantiation:** Pre-compile the new Wasm modules (utilizing the `InstancePre` cache if applicable).
3. **The Swap:** ```rust
   let new_runtime = Arc::new(IntegrityRuntime::new(new_config, ...));
   state.runtime.store(new_runtime);
   tracing::info!("Integrity configuration reloaded successfully");
   ```

## 3. WebSocket Persistence
- Ensure that the `system-faas-websocket` module uses a long-lived state that is independent of the `IntegrityRuntime` swap, so connections don't drop when a route is updated.