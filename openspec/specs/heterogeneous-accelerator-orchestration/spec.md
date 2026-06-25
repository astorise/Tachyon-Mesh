# Heterogeneous Accelerator Orchestration

## Purpose
Define how Tachyon orchestrates inference workloads across heterogeneous accelerator backends while preserving declared affinity and fallback behavior.

## Requirements
### Requirement: Model invocations are dispatched to heterogeneous accelerator backends according to declared affinity
The platform SHALL map configured models onto GPU, NPU, TPU, or CPU backends and SHALL respect fallback policy when a preferred backend is unavailable.

#### Scenario: A target binds models to multiple accelerator classes
- **WHEN** the host prepares execution for a target with heterogeneous model affinity
- **THEN** it dispatches each model to the declared accelerator backend
- **AND** applies the configured fallback behavior when a preferred backend cannot be used

### Requirement: NPU and TPU invocations MUST execute on real vendor backends
The platform SHALL provide at least one real NPU backend and one real TPU backend for documented model formats and operation subsets.

#### Scenario: OpenVINO NPU execution
- **GIVEN** a supported model and an initialized OpenVINO NPU device
- **WHEN** the model is invoked with NPU affinity
- **THEN** inference executes through OpenVINO on the NPU
- **AND** no silent CPU or GPU fallback is reported as NPU execution

#### Scenario: Coral Edge TPU execution
- **GIVEN** an Edge-TPU-compiled TFLite model and a detected Coral device
- **WHEN** the model is invoked with TPU affinity
- **THEN** inference executes through the Edge TPU delegate
- **AND** no silent CPU or GPU fallback is reported as TPU execution

### Requirement: NPU and TPU support MUST have physical acceptance evidence
The platform SHALL capture device discovery and execution evidence on labeled hardware before reporting either backend as supported.

#### Scenario: Hardware acceptance run
- **WHEN** the labeled NPU or Coral TPU validation job runs
- **THEN** it captures device discovery, backend selection, inference output, and fallback behavior
- **AND** a missing SDK or device fails the acceptance job rather than being skipped
