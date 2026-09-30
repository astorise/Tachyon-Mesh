## ADDED Requirements

### Requirement: GPU CI MUST validate real Magnetar CUDA execution without vacuous success
The GPU quality workflow SHALL validate the active Magnetar local inference path
on a self-hosted GPU runner. The job SHALL run a CUDA multi-token inference
Component test through Magnetar and SHALL fail if the selected test set is
empty or if the hardware-required assertion did not run.

#### Scenario: GPU job runs Magnetar CUDA multi-token proof
- **WHEN** the GPU quality job runs on the self-hosted NVIDIA runner
- **THEN** it executes a Magnetar Component CUDA generation test for at least sixteen tokens
- **AND** the test verifies CUDA availability through Magnetar
- **AND** the test fails if CUDA is unavailable on that runner

#### Scenario: Zero selected GPU tests fails CI
- **WHEN** the GPU workflow command selects tests by name or filter
- **THEN** the workflow verifies that at least one test is selected before reporting success
- **AND** a command that would execute zero tests fails the job

### Requirement: CPU CI MUST validate real Magnetar Component invocation
The standard quality workflow SHALL include non-GPU coverage for Tachyon's
active Magnetar Component integration. The coverage SHALL prove Tachyon-shaped
Component provenance, host-controlled trust evaluation, resident Component
materialization, and CPU generation through Magnetar.

#### Scenario: Quality job covers CPU production ingestion
- **WHEN** the quality workflow runs without GPU hardware
- **THEN** it executes a Tachyon-to-Magnetar Component invocation and CPU generation test
- **AND** the test does not depend on Candle runtime modules or fake Magnetar handles

## REMOVED Requirements

### Requirement: CI validates the optional AI inference build path
**Reason**: The old optional AI build path referenced legacy WASI-NN/Candle behavior as the active proof for local inference.
**Migration**: CI must validate the Magnetar Component path under `core-host/ai-inference` and retain any legacy WASI-NN checks only as explicit legacy compatibility coverage.
