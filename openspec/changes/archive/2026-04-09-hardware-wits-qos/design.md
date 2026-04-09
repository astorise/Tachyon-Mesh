# Design: Explicit Hardware WITs and QoS Scheduling

## Summary
Tachyon now exposes explicit accelerator host interfaces for component guests through a dedicated `tachyon:accelerator` WIT package while keeping the existing `tachyon:mesh` package stable. The AI runtime routes requests into per-accelerator schedulers and prioritizes them with QoS-aware queues.

## WIT Packaging
- The accelerator interfaces live in `wit-accelerator/` as four separate files: CPU, GPU, NPU, and TPU.
- The host binds them through a dedicated `host` world so the existing `faas-guest` world stays unchanged for current components.
- Linker registration is conditional: CPU is always linked, GPU only when the runtime exposes GPU-backed models, and NPU/TPU stay absent until the host actually supports them.

## QoS Model
- `IntegrityModelBinding` now accepts `qos` with `RealTime`, `Standard`, and `Batch`.
- QoS is attached to the bound model alias rather than the route target so both legacy `wasi-nn` guests and explicit accelerator component guests resolve the same scheduling priority from the selected model.
- If omitted, QoS defaults to `Standard`.

## Scheduler
- The Candle runtime maintains one scheduler per accelerator class.
- Each scheduler owns a `BinaryHeap` protected by `Mutex + Condvar`.
- Jobs sort first by QoS score, then by insertion order for FIFO behavior within equal priority.
- After every batch, remaining jobs are aged by incrementing their effective score to prevent starvation.
- Batching still groups compatible work by model alias, but the next batch is chosen from the global priority queue for that accelerator.

## Validation
- Added tests for alias preloading, concurrent batching, QoS preemption, and explicit component accelerator loading.
- Verified `cargo test -p core-host --features ai-inference`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo build`, and `openspec validate --all`.
