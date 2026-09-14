# tachyon-router Specification

## Purpose
Define how Tachyon routes inference traffic around Magnetar-backed local execution. Tachyon owns API ingress, mesh routing, placement policy, QoS, and stream transport; Magnetar owns model ingestion, tokenization, memory, execution plans, and Provider execution.
## Requirements
### Requirement: Magnetar Provider Metadata Advertisement
Tachyon-Mesh SHALL derive local inference capability advertisements from initialized Magnetar Providers and their public metadata instead of Tachyon-only environment variables, local fake device identifiers, or hardcoded memory/dtype tables.

#### Scenario: Node starts with CUDA provider
- **GIVEN** Magnetar initializes a CUDA provider on node startup
- **WHEN** Tachyon collects node capabilities
- **THEN** Tachyon SHALL record the provider's opaque device identifier
- **AND** Tachyon SHALL record Provider metadata exposed by Magnetar
- **AND** Tachyon SHALL mark unavailable Provider telemetry as unknown rather than fabricating it
- **AND** Tachyon SHALL publish those capabilities to mesh discovery

#### Scenario: Node starts with CPU fallback provider
- **GIVEN** Magnetar CUDA initialization is unavailable
- **WHEN** Tachyon initializes inference capabilities
- **THEN** Tachyon SHALL expose a reference CPU provider advertisement
- **AND** requests requiring GPU-only dtypes or placement SHALL be routed elsewhere when possible

### Requirement: Generation Stream Producer
Tachyon-Mesh SHALL stream local generation output from Magnetar's public generation events and propagate downstream cancellation to Magnetar. Tachyon SHALL NOT fabricate token streams by buffering a full generation and replaying it as one chunk.

#### Scenario: Streaming inference request is accepted
- **GIVEN** a request matches a node's advertised Magnetar capabilities
- **WHEN** the gRPC or REST inference layer accepts the request
- **THEN** Tachyon SHALL assign a session identifier
- **AND** Tachyon SHALL invoke Magnetar's public streaming generation API
- **AND** Tachyon SHALL stream generated token deltas from Magnetar back to the client asynchronously
- **AND** downstream disconnect SHALL stop the Magnetar generation callback path
