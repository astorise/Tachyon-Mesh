# Execution Tasks

- [x] **Task 1: The Purge**
  - [x] Remove or gate all `candle-*` dependencies from the active `core-host` inference feature path.
  - [x] Remove Candle-backed runtime modules from `core-host/src/ai_inference` or make them unreachable from the default Magnetar path.
  - [x] Fix resultant compilation breaks by routing Magnetar-targeted Qwen inference through a Magnetar-compatible facade.

- [x] **Task 2: Magnetar Bootstrapping**
  - [x] Avoid the non-existent `magnetar-runtime = "0.1.0"` crate and introduce an internal Magnetar-compatible facade until an official runtime crate is available.
  - [x] Implement engine initialization for Magnetar model bindings, using a configured CUDA provider and falling back to the reference CPU provider.
  - [x] Expose Magnetar-style capability advertisements through Tachyon's runtime metadata for mesh-wide hardware discovery.

- [x] **Task 3: The Qwen Pipeline**
  - [x] Restrict explicit Magnetar model admission to the Qwen family during the cutover.
  - [x] Connect Tachyon's safetensors artifact discovery path to the Magnetar facade with mmap validation.
  - [x] Wire token generation through Tachyon's existing buffered and streaming response contracts.

- [x] **Task 4: Zero-Copy Host Bindings**
  - [x] Refactor Tachyon's Magnetar execution boundary to solely handle `PreparedKernelId` and `TensorId`.
  - [x] Assert that no host code exposes `get_tensor_bytes`, `write_tensor_data`, `PreparedKernelId`, or `TensorId` raw tensor escape hatches in the current codebase.

- [x] **Task 5: End-to-End Validation**
  - [x] Cover a 2-node local Tachyon topology with deterministic runtime instances.
  - [x] Route a Qwen 3.5-style inference request from Node A to Node B (equipped with `CUDA_0`).
  - [x] Validate Magnetar admission and opaque-handle execution under the continuous batching facade.
  - [x] Add targeted unit coverage for Magnetar Qwen loading, capability advertisement, and non-Qwen rejection.
