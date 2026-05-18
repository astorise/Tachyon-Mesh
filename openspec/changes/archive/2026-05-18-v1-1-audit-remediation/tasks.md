# Implementation Tasks

- [x] **Task 1: Fix CDC Broadcaster Authentication Bypass**
  - Locate the dummy bearer token check in `systems/system-faas-cdc-broadcaster/src/lib.rs`.
  - Implement a fail-closed auth mechanism or proper `biscuit-auth` validation.
  - Remove/fix the unit test that asserted the bypass behavior.

- [x] **Task 2: Fix Memory Exhaustion Vectors (DoS)**
  - In `core-host/src/mesh/migration.rs`, limit the size of the `HashMap` in `SubspaceAccessTracker` (e.g., clear or reject when `len() > 10000`).
  - In `systems/system-faas-olap-engine/src/lib.rs`, add a size limit before JSON deserialization.

- [x] **Task 3: Enforce Rust Toolchain**
  - Create `rust-toolchain.toml` in the repository root specifying channel `1.95.0`.

- [x] **Task 4: Correct OpenSpec Audit Trail**
  - Open `openspec/changes/archive/2026-05-16-ai-constrained-decoding/tasks.md` and replace `[x]` with `[ ]`.
  - Open `openspec/changes/archive/2026-05-18-predictive-vram-orchestration/tasks.md` and replace `[x]` with `[ ]`.
  - Open `openspec/changes/archive/2026-05-17-quic-zero-copy-safetensors-replication/tasks.md` and replace `[x]` with `[ ]`.
  - Open `openspec/changes/archive/2026-05-17-baas-data-fabric/tasks.md` and replace `[x]` with `[ ]`.
  - Open `openspec/changes/archive/2026-05-17-business-canary-orchestration/tasks.md` and replace `[x]` with `[ ]`.

- [ ] **Task 5: Gate Unwired Stubs Behind Experimental Flag**
  - Add `experimental = []` to `[features]` in `core-host/Cargo.toml`.
  - Scan `core-host/src` for `#[allow(dead_code)]` applied to recent stubs (e.g., `store/mod.rs`, `telemetry/mod.rs`, `server_h3.rs`, `samplers.rs`).
  - Replace `#[allow(dead_code)]` with `#[cfg(feature = "experimental")]` where the item is genuinely unwired (test-referenced stubs keep `#[allow(dead_code)]` so the existing test coverage still compiles in the default profile).
