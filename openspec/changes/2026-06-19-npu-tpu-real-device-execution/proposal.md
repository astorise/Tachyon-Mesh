# Proposal: Real NPU/TPU Execution and Hardware Validation

## Why
`heterogeneous-accelerator-orchestration` already specifies that *"the platform SHALL map configured models onto GPU, NPU, TPU, or CPU backends"* and that fallback policy applies when a preferred backend is unavailable. In practice this is implemented as **capability-routing only**: the WIT hardware capability surface lets a target *declare* affinity for `npu`/`tpu`, and the host's dispatch logic picks a backend label — but `candle::Device` (the enum actually used to allocate tensors and run kernels) has no `Npu`/`Tpu` variant. There is no code path that can execute a single op on an NPU or TPU; "dispatch to NPU" can only ever mean "dispatch to the NPU label, then run on CPU/GPU anyway" or fail.

Additionally, `heterogeneous-accelerator-orchestration`'s own validation tasks (run on a machine with CPU+NPU+GPU; connect a Coral USB TPU; verify with `nvidia-smi`/`intel_gpu_top`) were never executed — there is no record of this orchestration running against real heterogeneous hardware.

## What Changes
1. **Device abstraction**: introduce a host-side accelerator abstraction that can represent and dispatch to NPU/TPU execution backends without requiring a `candle::Device` variant — most NPU/TPU runtimes (e.g., OpenVINO for Intel NPU, Edge TPU runtime for Coral) are accessed through their own SDK rather than through candle's CUDA/Metal/CPU device model, so this is a host-level dispatch boundary, not a candle fork.
2. **At least one real backend per class**: wire one concrete NPU runtime (e.g., OpenVINO) and one concrete TPU runtime (e.g., Coral Edge TPU via `libedgetpu`) end-to-end for a minimal supported op set (e.g., quantized INT8 inference for a small classification/embedding model), proving the dispatch path executes for real rather than only routing a label.
3. **Hardware validation**: execute the long-deferred validation tasks from `heterogeneous-accelerator-orchestration` on real CPU+NPU+GPU and Coral USB TPU hardware, and record the results (capability detection output, `nvidia-smi`/`intel_gpu_top` evidence) as part of this change's acceptance criteria rather than leaving them as open tasks indefinitely.
4. **Honest fallback semantics**: until a given accelerator class has a real backend wired, the host SHALL report it as `unavailable` rather than `available-but-untested`, so capability routing cannot select a backend that has never executed a single op.

## Non-Goals
- Does not attempt to support every NPU/TPU vendor; this proposal wires one reference implementation per class to prove the abstraction, with vendor expansion as follow-up work.
- Does not change the existing GPU/CPU dispatch paths.
- Does not cover NPU/TPU support for the model-parallelism work in the separate tensor/pipeline/expert-parallelism proposal — those remain GPU-only for now.

## Impact
- **Affected capability**: `heterogeneous-accelerator-orchestration` (delta below).
- **Affected code**: new accelerator dispatch abstraction in `core-host`, vendor SDK bindings (OpenVINO / Edge TPU), hardware capability discovery.
- **Risk**: requires physical test hardware (NPU-equipped machine, Coral USB TPU) for both implementation validation and CI — CI cannot fully cover this without a self-hosted hardware runner; mitigated by scoping CI to capability-detection/dispatch-logic unit tests and treating the physical execution proof as a manual/labeled-runner acceptance step, explicitly tracked rather than silently skipped.
