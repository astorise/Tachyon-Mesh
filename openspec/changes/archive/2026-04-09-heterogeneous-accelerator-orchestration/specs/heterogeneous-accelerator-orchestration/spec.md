## ADDED Requirements

### Requirement: Model invocations are dispatched to heterogeneous accelerator backends according to declared affinity
The platform SHALL map configured models onto GPU, NPU, TPU, or CPU backends and SHALL respect fallback policy when a preferred backend is unavailable.

#### Scenario: A target binds models to multiple accelerator classes
- **WHEN** the host prepares execution for a target with heterogeneous model affinity
- **THEN** it dispatches each model to the declared accelerator backend
- **AND** applies the configured fallback behavior when a preferred backend cannot be used
