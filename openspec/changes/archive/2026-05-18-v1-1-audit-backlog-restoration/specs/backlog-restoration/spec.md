# Specification: Backlog Restoration

## 1. Unarchiving Incomplete Proposals
To properly re-enqueue the un-wired scaffolding for future agentic implementation, their respective configuration directories must be relocated out of the `archive/` directory.
- **Action:** Move the following directories from `openspec/changes/archive/` to the active `openspec/changes/` directory:
  - `2026-05-18-predictive-vram-orchestration`
  - `2026-05-17-quic-zero-copy-safetensors-replication`
  - `2026-05-17-baas-data-fabric`
  - `2026-05-17-dynamic-geo-pinning`
  - `2026-05-17-cqrs-materialized-views`
  - `2026-05-17-baas-advanced-capabilities`
  - `2026-05-17-baas-ephemeral-compute`
  - `2026-05-17-compute-pushdown-wasm`

*(Note: `business-canary-orchestration` remains archived as it was successfully completed during the audit closure, and `ai-constrained-decoding` was actively deleted from the codebase).*