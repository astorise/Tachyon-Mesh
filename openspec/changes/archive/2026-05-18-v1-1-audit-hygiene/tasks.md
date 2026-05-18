# Implementation Tasks

- [x] **Task 1: Downgrade to Pre-release Version (1.1.0-alpha)**
  - Update version definitions to `1.1.0-alpha` across all `Cargo.toml` workspace files.
  - Update `version` in root `package.json`.
  - Update `version` and `package > version` keys in `tachyon-ui/tauri.conf.json`.

- [x] **Task 2: Fix Telemetry Mutex Poisoning Handling**
  - Locate `unwrap_or_else(|p| p.into_inner())` in `core-host/src/telemetry/mod.rs`.
  - Replace it with robust panic error propagation or fail-safe logic with visible crash logs.
