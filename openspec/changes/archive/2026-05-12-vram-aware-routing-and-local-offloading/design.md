# Design: VRAM-Aware Routing and Local Memory Offloading

## Approach

Four independent workstreams that share no runtime coupling. All operate through existing `AppState` references.

### 1. VRAM Tracking in `MemoryGovernor`

Two new atomic fields are added: `vram_utilization_pct: AtomicU8` and `ram_offload_active: AtomicBool`. The AI inference runtime calls `set_vram_utilization(pct)` whenever it refreshes accelerator metrics; the governor computes the derived offload flag (`pct >= 90`) atomically. `vram_pressure()` maps the percentage to the existing `MemoryPressure` enum: Normal (<80%), High (80-89%), Critical (≥90%). This reuses the enum already consumed by `execute_route_request` for system-RAM shedding.

**No disk/NVMe swap** is implemented per spec constraint. The "offload" flag is purely advisory to the host's KV-cache allocator; actual tensor pinning to host RAM via PCIe is performed by the Candle backend already in-process when `ram_offload_active` is set.

### 2. VRAM-Aware Admission (`enforce_vram_admission`)

Implemented as a standalone `pub(crate) fn enforce_vram_admission(state, route)` called from `execute_route_request` immediately after `enforce_resource_admission`, only for routes with declared AI models (`!route.models.is_empty()`).

- **Normal / High pressure** → returns `None` (request proceeds). High pressure logs a `tracing::debug!` that can be correlated with routing traces.
- **Critical pressure (≥90%)** → returns a 503 response with `Retry-After: 5` and `x-tachyon-reason: vram-saturated`. Local queuing is intentionally delegated to the existing `TimedOut` buffering path rather than being duplicated here.

### 3. Semantic Context Flattener

`SemanticContextFlattener` (added to `ai_inference.rs`) replaces depth-first JSON traversal with **semantic role extraction**:

1. It looks for a top-level `messages` array (the standard OpenAI-style context format).
2. For each message, it reads `role` and `turn_id` / `id` fields.
3. Cache keys are assigned by role prefix + monotonic user-turn counter + turn identifier:
   - System prompt → `sys:0:{id}`
   - User turn N → `usr:{N}:{turn_id}` (counter incremented per user turn)
   - Assistant response → `ast:{N}:{turn_id}`

This guarantees that lexicographic key ordering in the redb B-Tree mirrors conversational order, so `get_range("usr:1:", "usr:2:")` returns exactly the first user/assistant exchange without scanning unrelated entries.

Legacy (non-`messages`) payloads fall back to `legacy:0` to preserve backwards compatibility.

### 4. UI / Observability

**`AdminRuntimeMetrics`** gains `vramUtilizationPct: u8` and `ramOffloadActive: bool`, populated directly from `state.memory_governor`. The same fields are added (with `#[serde(default)]`) to the client-side `RuntimeMetrics` struct in `tachyon-client`, so existing callers that receive an older core-host response don't break.

**`TachyonAIPanel`** now fetches metrics on mount via `get_metrics` and renders:
- A VRAM utilization bar (cyan → amber → red as utilization rises past 80% / 90%)
- An amber "RAM Offload Active" badge with a PCIe spill explanation when `ramOffloadActive` is true
- A Refresh button for on-demand polling

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| VRAM source of truth | `MemoryGovernor` (call `set_vram_utilization`) | Separate `VramGovernor` | Reuses existing pressure enum + `AppState` reference; no new plumbing |
| Admission location | New `enforce_vram_admission` fn after existing admission | Inline in `execute_route_request` | Isolated, testable, clear separation of RAM vs VRAM policies |
| Local queuing | Reuse existing `TimedOut` buffer path | New dedicated VRAM queue | The existing path already provides a bounded local `await` queue |
| Cache key scheme | `{role}:{turn}:{id}` prefix | SHA/hash of content | Lexicographic ordering in redb is free; content hashes break range queries |
| Disk swap | Not implemented | NVMe spill | Spec constraint: sequential read latency of inference generation forbids NVMe swap |
