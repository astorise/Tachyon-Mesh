## ADDED Requirements

### Requirement: Mesh AI capability routing MUST use real Magnetar Provider metadata
For AI model placement, the host SHALL base mesh-visible Provider and Device capabilities on Magnetar Provider metadata and status. Tachyon SHALL retain distributed routing, QoS, and provenance decisions, but it SHALL NOT fabricate Magnetar hardware capability data from local constants or environment variables.

#### Scenario: Node advertises Magnetar CPU capability
- **WHEN** a node enables local AI inference with Reference CPU placement
- **THEN** mesh capability data identifies the capability as coming from Magnetar Reference CPU Provider metadata
- **AND** Tachyon does not advertise hardcoded fake VRAM or CUDA identifiers for that CPU path

#### Scenario: Node advertises Magnetar CUDA capability
- **WHEN** a node enables local AI inference with CUDA placement and a real Magnetar `CudaProvider` is available
- **THEN** mesh capability data identifies the CUDA Provider and Device using Magnetar-derived metadata
- **AND** Tachyon does not treat `TACHYON_MAGNETAR_CUDA` or any other Tachyon-only environment variable as proof of hardware availability

#### Scenario: CUDA unavailable is explicit
- **WHEN** a route requires CUDA and Magnetar reports no available CUDA Provider
- **THEN** placement fails with a structured unsupported-capability or unavailable-provider result
- **AND** the request is not silently rerouted to Reference CPU
