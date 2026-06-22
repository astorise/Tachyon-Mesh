# Implementation Tasks

- [ ] **Task 1: AcceleratorBackend abstraction**
  - Introduce `AcceleratorBackend` (`Candle(Device) | OpenVinoNpu | EdgeTpu`) and `AcceleratorAvailability { backend, status }` in `core-host`.
  - Wire existing GPU/CPU dispatch through `AcceleratorBackend::Candle` with no behavior change.

- [ ] **Task 2: OpenVINO NPU backend**
  - Add the OpenVINO SDK binding (feature-gated, e.g. `npu-openvino`).
  - Implement device enumeration and INT8 IR model load/inference for the minimal supported op set.

- [ ] **Task 3: Edge TPU backend**
  - Add the `libedgetpu` binding (feature-gated, e.g. `tpu-edgetpu`).
  - Implement device enumeration and `.tflite` model load/inference via the Edge TPU delegate.

- [ ] **Task 4: Capability reporting tied to real backend presence**
  - Update `heterogeneous-accelerator-orchestration`'s capability discovery so `npu`/`tpu` availability reflects successful backend initialization, not just declared affinity.
  - Ensure targets declaring `npu`/`tpu` affinity without a wired/initialized backend fall back per existing policy instead of routing to a non-functional label.

- [ ] **Task 5: Hardware validation (manual / labeled hardware runner)**
  - Run on a real CPU+NPU+GPU machine: verify capability discovery and dispatch/fallback behavior; capture output.
  - Connect a Coral USB TPU: verify detection and dispatch; capture output.
  - Cross-check with `nvidia-smi`/`intel_gpu_top` (and OpenVINO/Edge TPU equivalents) that the declared backend executed the work.
  - Attach captured evidence to this change's record.

- [ ] **Task 6: Tests**
  - Unit tests for `AcceleratorAvailability` status transitions (available / unavailable + reason) without requiring physical hardware.
  - CI-runnable dispatch-logic tests using a mocked `AcceleratorBackend` to confirm fallback policy when NPU/TPU is `Unavailable`.
  - Hardware-gated integration tests (skipped in standard CI, run on a labeled hardware runner) for the OpenVINO and Edge TPU minimal op set.

- [ ] **Task 7: Docs**
  - Document supported NPU/TPU vendors and the minimal op set per class.
  - Update `heterogeneous-accelerator-orchestration` operator docs to state which accelerator classes have a real backend vs. capability-routing-only.
