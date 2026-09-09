# tachyon-router Specification

## Purpose
TBD - created by archiving change magnetar-full-cutover. Update Purpose after archive.
## Requirements
### Requirement: Magnetar Capability Advertisement
Tachyon-Mesh SHALL derive inference hardware capabilities from Magnetar provider advertisements instead of direct accelerator probing in routing code.

#### Scenario: Node starts with CUDA provider
- **GIVEN** Magnetar initializes a CUDA provider on node startup
- **WHEN** Tachyon collects node capabilities
- **THEN** Tachyon SHALL record the provider's opaque device identifier
- **AND** Tachyon SHALL record the provider's available memory
- **AND** Tachyon SHALL record the provider's supported dtypes
- **AND** Tachyon SHALL publish those capabilities to mesh discovery

#### Scenario: Node starts with CPU fallback provider
- **GIVEN** Magnetar CUDA initialization is unavailable
- **WHEN** Tachyon initializes inference capabilities
- **THEN** Tachyon SHALL expose a reference CPU provider advertisement
- **AND** requests requiring GPU-only dtypes or placement SHALL be routed elsewhere when possible

### Requirement: ExecutionStream Producer
Tachyon-Mesh SHALL enqueue token generation requests into Magnetar's continuous batching execution stream.

#### Scenario: Streaming inference request is accepted
- **GIVEN** a request matches a node's advertised Magnetar capabilities
- **WHEN** the gRPC or REST inference layer accepts the request
- **THEN** Tachyon SHALL assign a session identifier
- **AND** Tachyon SHALL append the request to Magnetar's execution stream
- **AND** Tachyon SHALL stream generated tokens from Magnetar back to the client asynchronously

