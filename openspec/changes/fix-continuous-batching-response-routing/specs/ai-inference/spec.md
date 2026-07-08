## MODIFIED Requirements

### Requirement: Inference requests are continuously batched by the host
The host SHALL run a continuous batching scheduler that admits compatible inference sequences into
an active set, advances each sequence through explicit `prefill` and `decode` phases, and chooses
the next compatible step without waiting for a fixed time window. When multiple requests are
processed in the same batched step, each request's own generated output SHALL be routed back to
that request's own caller — never another request's output, and never silently dropped in favor of
processing only the first request in the batch.

#### Scenario: Compatible inference requests are active together
- **WHEN** several compatible inference requests are active on the same accelerator
- **THEN** the scheduler groups their next matching phase into a shared prefill or decode step
- **AND** routes each generated response back to the correct caller

#### Scenario: New work is admitted while decode is in flight
- **WHEN** a new higher-QoS inference request arrives while another sequence is already active
- **THEN** the scheduler may admit the new request into the active set before the existing sequence completes
- **AND** the next eligible step is selected by QoS and compatibility rather than by the original arrival batch

#### Scenario: A shared decode batch contains distinct prompts for the same model
- **GIVEN** two or more inference requests for the same model alias and adapter are grouped into
  the same decode batch
- **AND** the requests carry different prompts
- **WHEN** the scheduler processes that batch
- **THEN** each request's response is generated from that request's own prompt
- **AND** no request receives another request's generated output
- **AND** a backend that cannot produce a distinct output per request in the batch fails the batch
  with an error rather than silently returning fewer outputs than requests
