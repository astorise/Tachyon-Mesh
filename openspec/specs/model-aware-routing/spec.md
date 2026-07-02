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

### Requirement: Model alias extraction avoids parsing non-AI request bodies
The host SHALL extract model aliases from request bodies only for routes that declare model bindings. Header-based aliases such as `x-tachyon-model` SHALL remain available for every route, but non-AI routes SHALL NOT parse or clone full JSON request bodies only to evaluate model-aware routing.

#### Scenario: Non-AI route carries a JSON body with a model field
- **GIVEN** a sealed route without configured `models`
- **WHEN** the request body contains a JSON `model` field
- **THEN** the host does not parse the body for model-aware routing
- **AND** it does not derive a requested model alias from that body field

#### Scenario: AI route selects a model alias from the request body
- **GIVEN** a sealed route with multiple configured `models`
- **WHEN** the request body contains a JSON `model`, `model_alias`, or `alias` string
- **THEN** the host may use that field to select the requested model alias without cloning the full JSON object map
