## ADDED Requirements

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
