# Design: v1.1.x Audit Backlog Restoration

## What Was Built

Pure filesystem restoration — the eight OpenSpec changes whose tasks were correctly reset to `[ ]` during the audit cycle are now un-archived so the agentic workflow re-enqueues them as live technical debt.

## Movements

All eight directories moved from `openspec/changes/archive/` back to `openspec/changes/`:

| Group | Directory |
|---|---|
| VRAM & QUIC (Task 1) | `2026-05-18-predictive-vram-orchestration` |
|  | `2026-05-17-quic-zero-copy-safetensors-replication` |
| BaaS (Task 2) | `2026-05-17-baas-data-fabric` |
|  | `2026-05-17-baas-advanced-capabilities` |
|  | `2026-05-17-baas-ephemeral-compute` |
| Infrastructure (Task 3) | `2026-05-17-dynamic-geo-pinning` |
|  | `2026-05-17-cqrs-materialized-views` |
|  | `2026-05-17-compute-pushdown-wasm` |

Each directory retains its dated prefix (e.g. `2026-05-17-…`), preserving the original proposal date for audit traceability while now living in the active queue.

## Why This Is Administrative-Only

No code is touched in this change. The implementation work for each of these eight proposals lands in future `v1.1.x` minor cycles or `v1.2.0`. This change exists solely to **re-enqueue** them — without this move, the agentic OpenSpec workflow treats the items as closed (since they're in `archive/`) and would never schedule them for completion.

## Verification

`openspec list` will now show the 8 restored changes alongside any other active items. Each archived task that had been reset to `[ ]` in `v1-1-ga-readiness` Task 1 now sits inside an active change directory, so the unfinished work is visible to the agentic loop.
