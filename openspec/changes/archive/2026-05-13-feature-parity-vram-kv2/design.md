# Design: feature-parity-vram-kv2

## Overview

Adds GPU VRAM observability to the hardware panel, a KV-Partition V2 explorer to the storage panel, extends the hardware status struct to carry per-GPU topology data for MCP agents, and aligns the manifest JSON Schema with the new `vram_mb` / `gpu_affinity` resource policy fields.

## Task 1 — VRAM UI Component (`TachyonHardwarePanel.ts`)

The hardware panel now fetches both `get_hardware_status` and `get_metrics` in parallel. A new `renderVramSection()` method produces a `data-stagger-panel` block containing:

- **Per-GPU progress bars** — when `HardwareStatus.gpus` is non-empty, each GPU entry renders a labelled progress bar showing `vramUsedMb / vramTotalMb`. While VRAM values are zero (no GPU management library), the bar falls back to the cluster-wide `vramUtilizationPct` percentage.
- **Cluster-wide VRAM bar** — derived from `RuntimeMetrics.vramUtilizationPct`, coloured cyan < 80 %, amber 80–90 %, red ≥ 90 %.
- **RAM offload badge** — rendered when `ramOffloadActive` is true, matching the AI panel indicator.
- **Refresh button** — calls `get_metrics` and re-renders the VRAM section without a full page reload.

The TypeScript `HardwareStatus` type is extended with `gpus: GpuStats[]`, and a local `RuntimeMetrics` type (subset) is added.

## Task 2 — KV Explorer (`TachyonStoragePanel.ts`)

A new **KV-Partition V2 Explorer** section is added to the storage panel above the existing volume config form. It provides:

- **Namespace** and **Key** inputs (text, monospace).
- **Get** button — calls `invoke("kv_get", { namespace, key })`. On success the value is pretty-printed if it parses as JSON, otherwise shown raw. On 404 / missing key "(key not found)" is displayed.
- **Delete** button — calls `invoke("kv_delete", { namespace, key })` and dispatches a `toast` success event.
- **Inline result zone** — rendered separately via `renderKvResult()` so it updates without re-rendering the whole template. Includes a dismiss (✕) button.
- Both buttons disable during in-flight requests (`kvBusy` flag).

The component subscribes to `i18n:language-changed` and re-renders on locale switches.

## Task 3 — MCP Hardware Payload (`tachyon-client/src/lib.rs`)

`HardwareStatus` gains a new `gpus: Vec<GpuStats>` field (serialised as `"gpus"` in camelCase JSON). `GpuStats` carries: `id`, `model`, `vramTotalMb`, `vramUsedMb`, `computeUtilization`.

`read_local_hardware_status()` now inspects `CUDA_VISIBLE_DEVICES` (NVIDIA) and `HIP_VISIBLE_DEVICES` (AMD). It enumerates the comma-separated device IDs and produces one `GpuStats` entry per device. VRAM values default to 0 because no GPU management library is linked; they will be populated by the AI inference runtime once it updates the memory governor. The MCP `tachyon_hardware_status` tool automatically includes GPU topology data in its response since it serialises `HardwareStatus` verbatim.

## Task 4 — Schema Alignment (`domain_types.rs` + `app_runtime.rs`)

`ResourcePolicy` gains two new fields:
- `vram_mb: Option<u64>` — GPU VRAM reservation in MiB, scheduler-enforced.
- `gpu_affinity: Option<String>` — GPU device affinity selector (e.g. `"cuda:0"`, `"RTX 4090"`).

The handcrafted JSON Schema in `admin_manifest_schema_handler` is updated to document both fields under `resourcePolicy.properties`, with descriptions explaining scheduler semantics. `vramMb` / `gpuAffinity` are now surfaced to MCP agents via the `GET /admin/schema/manifest` endpoint and injected into the `tachyon_dryrun_manifest` tool schema.
