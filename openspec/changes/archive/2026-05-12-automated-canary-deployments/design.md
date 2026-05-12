# Design: Automated Canary Deployments for WASM Components

## Approach

A three-layer implementation: WIT contract, Rust host runtime, and Tauri UI.

### 1. WIT Contract Extension

`wit/config-workloads.wit` gains a `deployment-strategy` variant (rolling | canary) and a `canary-config` record mirroring the new `CanaryConfig` Rust struct. `workload-spec` is extended with optional `strategy` and `canary` fields so WASM config components can expose these choices to the UI.

### 2. Domain Types & IntegrityRoute

`CanaryConfig` (serde-deserializable) is added to `domain_types.rs` and attached as an optional `canary` field on `IntegrityRoute`. The canary runtime state (`CanaryRolloutState`) is separate from the config: it holds `AtomicU32 weight_pct`, per-rollout error counters (`AtomicU64`), and a `watch::Sender<bool>` for evaluator lifecycle management. A module-level `CANARY_ROLLOUTS: OnceLock<Arc<Mutex<HashMap<...>>>>` global provides a single shared registry accessible from the routing hot path and the evaluator tasks.

### 3. Fractional Routing

In `execute_route_request` (app_runtime.rs), after `select_route_module` resolves the default module, the code performs a lock-free read of `CANARY_ROLLOUTS`. If an active `Stepping` rollout exists for the route, a `rand::rng().random_range(0u32..100)` draw is compared against `weight_pct`: hits are sent to `next_version`, misses keep the current module. The `Ok(permit)` execution branch increments per-rollout request and error counters from the result status code. Overflow/buffered paths are excluded from canary accounting (they forward to peer nodes, not the local canary module).

### 4. Telemetry-Driven Evaluator

`spawn_canary_evaluators(config)` (background_workers.rs) stops any existing evaluators (via `stop_tx`), rebuilds the `CANARY_ROLLOUTS` registry from the new config, and spawns one async tokio task per canary-enabled route. Each `run_canary_evaluator` task sleeps for `interval_secs`, then computes `error_rate = next_err_count / next_req_count`. If the rate exceeds `max_error_rate`, the evaluator atomically sets `weight_pct = 0`, transitions to `RolledBack`, and emits a `tracing::error!` critical event (surfaced in the shadow-diffs/events log). Otherwise it increments the weight by `step_weight`; at 100% it transitions to `Promoted`. `spawn_canary_evaluators` is called at startup (`entrypoint.rs`) and on every hot reload (`supervisors.rs`).

### 5. Admin Surface

`GET /admin/canary` returns a JSON array of `CanaryStatusEntry` (route, versions, phase, weight, counters). `POST /admin/canary` accepts `{ "routePath": "..." }` and aborts the named rollout. Both are auth-guarded by the existing admin middleware. The client-side Tauri command `fetch_canary_status` and `abort_canary_rollout` wrap these endpoints.

### 6. UI (TachyonWorkloadsPanel)

The deployment form gains a **Strategy** selector. When "Canary" is selected, a highlighted block of four config inputs appears (next version, step weight, interval, max error rate). The payload sent via `applyAndSeal` includes a `canary` object matching the `CanaryConfig` serde shape. Below the form, an **Active Canary Rollouts** section polls `fetch_canary_status`, renders a per-route amber progress bar (0–100% traffic split), and shows phase badges. An **Abort Rollout** button fires `abort_canary_rollout` with a confirmation prompt.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Canary state storage | `CANARY_ROLLOUTS` global (like `LORA_TRAINING_QUEUE`) | Field in `AppState` | Avoids plumbing AppState through `select_route_module`; consistent with existing pattern |
| Error tracking | Per-rollout AtomicU64 counters updated in `Ok(permit)` branch | Global telemetry delta | Spec requires per-`next_version` rate; global delta misses attribution |
| Overflow path tracking | Skipped | Tracked | Overflow goes to a peer node — charging errors against the local canary is incorrect |
| Evaluator lifecycle | `watch::Sender<bool>` stop signal | `tokio::CancellationToken` | `watch` is already imported; avoids a new dependency |
| Abort API | `POST /admin/canary` with JSON body | `DELETE /admin/canary/:path` | Route paths may contain slashes; JSON body avoids URL-encoding complexity |
