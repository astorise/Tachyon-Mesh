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
