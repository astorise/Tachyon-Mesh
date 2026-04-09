# Model-Aware Routing

## Purpose
Define how Tachyon uses hot-model telemetry to keep latency-sensitive inference on peers that already have the requested model resident, avoiding cold remote loads that would dominate queueing time.

## Requirements
### Requirement: AI routing prefers peers that already have the target model hot
The router SHALL consider hot-model state as a first-class placement signal so latency-sensitive inference is not sent to peers that would incur a cold load.

#### Scenario: A real-time request targets a model that is not hot on a remote peer
- **WHEN** the router evaluates overflow candidates for a latency-sensitive model invocation
- **THEN** it keeps the request local or selects only peers that already have the target model loaded
- **AND** avoids sending that request to peers that would need a cold model load

#### Scenario: A matching hot peer exists even if a colder peer looks less busy
- **WHEN** the router evaluates model-aware overflow candidates for a request that names a specific model alias
- **THEN** it prefers peers whose advertised hot-model list contains that alias
- **AND** only forwards to a lower-pressure peer when that peer is also hot for the requested model
