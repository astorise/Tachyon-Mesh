# heterogeneous-accelerator-orchestration Delta

## MODIFIED Requirements

### Requirement: Model invocations are dispatched to heterogeneous accelerator backends according to declared affinity
The platform SHALL map configured models onto GPU, NPU, TPU, or CPU backends and SHALL respect fallback policy when a preferred backend is unavailable. The platform SHALL only report an accelerator class as available when a real execution backend for that class has been successfully initialized against detected hardware; declared affinity alone SHALL NOT make an accelerator class available.

#### Scenario: A target binds models to multiple accelerator classes
- **WHEN** the host prepares execution for a target with heterogeneous model affinity
- **THEN** it dispatches each model to the declared accelerator backend
- **AND** applies the configured fallback behavior when a preferred backend cannot be used

#### Scenario: NPU affinity without a wired backend falls back instead of routing to a non-functional label
- **GIVEN** a target declares `npu` affinity
- **AND** no NPU execution backend has been successfully initialized on the host
- **WHEN** the host dispatches a model invocation for that target
- **THEN** the host reports the NPU accelerator class as unavailable
- **AND** applies the configured fallback policy instead of dispatching to a label with no real execution path

## ADDED Requirements

### Requirement: NPU and TPU model invocations MUST execute on a real accelerator backend
For at least one supported NPU runtime and one supported TPU runtime, the platform SHALL execute model inference through that vendor's native SDK, producing real output, for a defined minimal supported model/op set.

#### Scenario: NPU-affine model executes via the OpenVINO backend
- **GIVEN** a target declares `npu` affinity for a model in the supported NPU op set
- **AND** an OpenVINO-compatible NPU device is detected and initialized
- **WHEN** an inference request is dispatched for that model
- **THEN** the host executes the model through the OpenVINO Inference Engine on the detected NPU
- **AND** returns real inference output, not a CPU/GPU fallback result

#### Scenario: TPU-affine model executes via the Edge TPU backend
- **GIVEN** a target declares `tpu` affinity for a `.tflite` model compiled for Edge TPU
- **AND** a Coral USB Edge TPU is detected and initialized
- **WHEN** an inference request is dispatched for that model
- **THEN** the host executes the model through the `libedgetpu` delegate on the detected Edge TPU
- **AND** returns real inference output, not a CPU/GPU fallback result

### Requirement: Heterogeneous accelerator orchestration MUST be validated against real hardware
The platform's NPU/TPU/GPU capability detection and dispatch/fallback behavior SHALL be validated on physical hardware combining CPU, NPU, and GPU, and separately with a connected Coral USB TPU, with captured evidence of correct backend selection.

#### Scenario: Mixed CPU+NPU+GPU hardware validation
- **WHEN** the orchestration runs on a machine exposing CPU, NPU, and GPU accelerators
- **THEN** capability discovery correctly enumerates all three
- **AND** model dispatch honors declared affinity and falls back correctly when a preferred backend is busy or unavailable
- **AND** evidence of correct backend selection (e.g., `nvidia-smi`/`intel_gpu_top`/OpenVINO device logs) is captured

#### Scenario: Coral USB TPU validation
- **WHEN** a Coral USB TPU is connected to the host
- **THEN** capability discovery detects it
- **AND** a model declared with `tpu` affinity executes via the Edge TPU backend
- **AND** evidence confirms the Edge TPU, not CPU/GPU, performed the inference

## Implementation status as of this change

Only the first MODIFIED requirement ("dispatched according to declared affinity" /
honest-unavailable-reporting) is implemented, and only at the host-level model-load
boundary, not the two ADDED requirements (real OpenVINO/Edge TPU execution, physical
hardware validation):

- **`core-host/src/ai_inference/accelerator_backend.rs`** (new module) implements
  `probe(AcceleratorKind) -> AcceleratorAvailability`: `Cpu` is always available;
  `Gpu` is probed for real via `parallel::discover_cluster_topology` (existing CUDA
  device enumeration); `Npu`/`Tpu` always report `Unavailable { reason:
  "no_backend_wired" }`, since no vendor SDK backend is wired into the host. This
  satisfies "only report an accelerator class as available when a real execution
  backend ... has been successfully initialized" for `Npu`/`Tpu` (trivially — they
  are never claimed available) and for `Gpu` (genuinely probed, not assumed).
- **`AiInferenceRuntime::load_component_model`** (`core-host/src/ai_inference.rs`)
  calls `accelerator_backend::probe` and rejects the load with a typed error when,
  and only when, the requested accelerator is `Npu` or `Tpu` and unavailable. `Cpu`/
  `Gpu` dispatch in this function is unchanged from before this change — this was a
  deliberate scope decision to avoid altering already-tested GPU/CPU behavior (see
  `tasks.md` Task 1/4 notes).
  - This is the load-time gate, not full fallback routing: `resolve_with_fallback`
    exists and is unit-tested, but is not yet wired into the live WIT-facing
    `load_accelerator_model`/`compute_accelerator_prompt` dispatch path in
    `component_hosts.rs`. A target that declares `npu` affinity today gets a clear
    rejection at model-load time, not yet an automatic redirect to a fallback
    accelerator end-to-end through the guest-visible API.
  - `AiInferenceRuntime::supports_accelerator` (the pre-existing scheduling-lane/
    QoS-queue-sizing concept, unrelated to real hardware presence) is deliberately
    left unchanged: it still reports `true` for all four `AcceleratorKind` values,
    since per-class request-queue provisioning is a different question from "is
    there a real backend," and an existing test
    (`component_accelerator_runtime_rejects_mismatched_devices`) already asserts
    that lane-availability semantics for `Gpu`/`Npu`/`Tpu` in CPU-only CI.

The two **ADDED** requirements above — real OpenVINO NPU execution, real Edge TPU
execution via `libedgetpu`, and physical hardware validation on a CPU+NPU+GPU
machine and a Coral USB TPU — are **not implemented** by this change. They require
vendor SDK dependencies (OpenVINO, `libedgetpu`) and physical test hardware that are
not available in this sandboxed environment. `accelerator_backend.rs`'s module doc
comment and `tasks.md` Tasks 2/3/5/7 record this explicitly as deferred follow-up
work, rather than the gap being silently merged as spec text with no implementation
(the failure mode this change exists to avoid repeating, per
`2026-06-19-constrained-decoding-activation`'s precedent for an honest "implemented
vs. not" accounting).

Verified by 3 new unit tests in `accelerator_backend.rs` covering `probe` status
transitions, 3 new unit tests covering `resolve_with_fallback` (including an
injected-probe test demonstrating the fallback logic is independent of real
hardware), 1 new integration test in `ai_inference.rs`
(`load_component_model_honestly_rejects_npu_and_tpu_as_unavailable`), and the full
`core-host --features ai-inference` suite (121 tests, 0 regressions), plus a clean
`cargo clippy --features ai-inference --all-targets -- -D warnings -D
clippy::unwrap_used`.
