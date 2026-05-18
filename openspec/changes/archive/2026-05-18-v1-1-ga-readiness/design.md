# Design: v1.1 GA Readiness & Anti-Gaming

## What Was Built

Four anti-gaming corrections forcing architectural honesty before the GA cut.

### Task 1 — Truthful Audit Trail

Six archived `tasks.md` files had `[x]` boxes for work that was never actually implemented. Reset them to `[ ]` so the audit trail reflects reality:

| Archived change | Task reset to `[ ]` | Why |
|---|---|---|
| `2026-05-17-baas-ephemeral-compute` | Task 4 (system-faas-olap-engine) | Stub-only, no real columnar aggregator |
| `2026-05-17-compute-pushdown-wasm` | Task 4 (Logical Plane PoC) | Helpers exist behind `experimental`; no FaaS actually uses them |
| `2026-05-18-v1-1-audit-remediation` | Task 5 (Gate stubs behind experimental) | The flag existed but nothing was gated |
| `2026-05-18-v1-1-audit-full-closure` | Task 4 (Host-Guest integration test) | Was a grep-based fake |
| `2026-05-18-v1-1-audit-absolute-polish` | Task 4 (Integration test suite) | Same — all four scaffolded tests were `Path::exists()` checks |
| `2026-05-18-v1-1-audit-absolute-polish-v2` | Task 1 (Complete integration test suite) | Same |

The actual implementation of these tasks lands in **this** change (Tasks 2-3) and in `v1-1-audit-backlog-restoration`.

### Task 2 — Real `#[cfg(feature = "experimental")]` Gating

The audit found 53 stale `#[allow(dead_code)]` annotations across `core-host/src/`. A blanket swap to `#[cfg(feature = "experimental")]` initially produced 83 compilation errors — confirming that many of those annotations were silencing **legitimate** code that's reached via tool-chain paths invisible to the dead-code lint.

The implementation applied a compiler-guided three-way classification:

1. **True experimental stubs** → `#[cfg(feature = "experimental")]` so they vanish from the default build:
   - `core_error::{CoreError, CoreResult, poisoned_lock}` (typed-error infrastructure for v1.2)
   - `error::{TachyonError, TachyonResult}` (same)
   - `mesh::migration::{GeoMigrationPlan, DEFAULT_REMOTE_RATIO, DEFAULT_MIN_REMOTE_HITS, DEFAULT_COOLDOWN, MAX_TRACKED_SUBSPACE_PEERS}`
   - `state::{lock, RuntimeGenerationState}`
   - `store::{COMPRESSED_MAGIC_BYTES, BaasQueryCache, VersionedRecord, ConflictState, ScanOptions, should_compress_blob, pipe_range_from_file*, transform_read_if_stale, detect_split_brain, resolve_conflict_with, pushdown_wasmtime_config, execute_filtered_scan}`
   - `telemetry::{init_telemetry, metric_type, register_collector, normalize_metric_name, normalize_label_name, collector_key, validate_prometheus_ident}`
   - `auth::AuthzPurgeEvent::enqueue` + its unit test
   - `MODULE` tracing-target constants across `identity`, `mesh`, `network`, `runtime`, `storage` modules
   - `network::ebpf::EbpfFastPathStatus::Loaded` (gated to `ebpf-loader` instead — pre-existing constraint)
   - All in-module unit tests that exercise the gated items (now `#[cfg(all(test, feature = "experimental"))]`)

2. **Tool-chain false positives** (lint can't see the use site) → `#[allow(dead_code)]` with an inline justification comment:
   - All 38 `#[utoipa::path(...)]` stub functions in `openapi.rs` — referenced via `#[derive(OpenApi)] paths(...)` macro expansion
   - `WorkspaceGraphResource`, `RedbTableResource` — constructed and stored type-erased in a Wasmtime `ResourceTable`
   - `GraphEdge`, `GRAPH_SEP`, `GRAPH_TRAVERSE_LIMIT`, `graph_spo_key`, `graph_osp_key`, `graph_spo_prefix_range`, the `impl CoreStore` graph methods — reached via the `HostWorkspaceGraph` WIT trait dispatch
   - `CoreStore::{kv_partition_batch_set, kv_partition_get_range}` — reached via the `HostRedbTable` WIT trait dispatch
   - `AppState::subscribe_config_updates` — used by `integrity_admin.rs` integration test
   - `AiInferenceJobStatus::Failed` — produced by serde Deserialize, never constructed in Rust code

3. **Items actually used in default builds** → annotation removed entirely (compiler is happy):
   - `CustomMetric`, `CustomMetricType`, `push_custom_metric` — wired into Wasmtime host bindings (`component_hosts.rs`)
   - `CoreStoreBucket::{VectorIndices, AuthzPurgeOutbox, DataMutationOutbox}` enum variants — used by the outbox subsystem
   - `CustomCollector` enum + `PROMETHEUS_COLLECTORS` static + `CUSTOM_METRIC_HELP` const — used by `push_custom_metric`

Every `#[allow(dead_code)]` that survives now carries an inline comment explaining the specific tool-chain limitation it works around. The auditor's "no placebo" rule is enforced: there are no bare `#[allow(dead_code)]` left.

### Task 3 — Real Wasmtime Integration Test

Deleted the seven grep-based fake tests:

- `cdc_broadcaster_test.rs`, `host_guest_integration_test.rs`, `media_server_test.rs`, `olap_engine_test.rs`, `sql_engine_test.rs`, `vector_search_test.rs`, `view_builder_test.rs`

Replaced with one **authentic** integration test at [`core-host/tests/real_wasm_integration_test.rs`](core-host/tests/real_wasm_integration_test.rs) containing two tests:

1. **`wasmtime_engine_compiles_and_runs_inline_module`** — builds a real `wasmtime::Engine` with `consume_fuel(true)`, compiles inline WAT (a module that imports a host `record_hit` function and exports a `run` function), wires up a `Linker` with the host import, instantiates the module, calls `run`, and asserts the host-side counter incremented. Proves Engine → Linker → instantiation → dispatch is wired correctly.

2. **`wasmtime_engine_traps_on_fuel_exhaustion`** — builds an infinite-loop WAT module, sets a 1000-fuel budget, asserts the call traps. Proves the fuel-metering integration (critical for the pushdown-filter sandbox contract).

Both tests pass on stock `cargo test -p core-host --test real_wasm_integration_test`.

### Task 4 — Consistent Mutex Poisoning Logging

Aligned `store::BaasQueryCache::get_or_insert_with` with the established `telemetry::recover_poisoned` pattern. Each `lock().unwrap_or_else(...)` now logs a `tracing::warn!` with `target: "tachyon::store"` and a `registry` field before returning the recovered guard. Two write-paths are instrumented:
- Read-side: `"store mutex was poisoned on read; continuing with recovered guard"`
- Write-side: `"store mutex was poisoned on write; continuing with recovered guard"`

No more silent corruption — every poison event leaves an audit trail.

## Verification

- `cargo build -p core-host` (default features): clean, zero warnings.
- `cargo build -p core-host --features experimental`: builds (49 dead-code warnings on experimental items themselves; CI only checks default).
- `cargo clippy -p core-host --all-targets -- -D warnings -D clippy::unwrap_used`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo fmt --all -- --check`: clean.
- `cargo test -p core-host --test real_wasm_integration_test`: **2/2 passing**.

## Files Changed

**Source files** — annotation tightening:
- `core-host/src/{auth.rs, ai_inference.rs, core_error.rs, error.rs, mesh/migration.rs, mesh/mod.rs, network/mod.rs, runtime/mod.rs, state/mod.rs, storage/mod.rs, identity/mod.rs, server_h3.rs, store/mod.rs, telemetry/mod.rs}`
- `core-host/src/host_core/{domain_types.rs, graph_store.rs, openapi.rs, runtime_types.rs}`

**Tests** — deletions + creation:
- Deleted: `core-host/tests/{cdc_broadcaster, host_guest_integration, media_server, olap_engine, sql_engine, vector_search, view_builder}_test.rs`
- Created: `core-host/tests/real_wasm_integration_test.rs`

**Archive resets**:
- `openspec/changes/archive/2026-05-17-baas-ephemeral-compute/tasks.md`
- `openspec/changes/archive/2026-05-17-compute-pushdown-wasm/tasks.md`
- `openspec/changes/archive/2026-05-18-v1-1-audit-remediation/tasks.md`
- `openspec/changes/archive/2026-05-18-v1-1-audit-full-closure/tasks.md`
- `openspec/changes/archive/2026-05-18-v1-1-audit-absolute-polish/tasks.md`
- `openspec/changes/archive/2026-05-18-v1-1-audit-absolute-polish-v2/tasks.md`
