# shadow-traffic Specification

## Purpose
Define shadow traffic execution for testing new FaaS modules with production-shaped requests without affecting client responses.

## Requirements
### Requirement: Asynchronous shadow dispatch
The host SHALL dispatch shadow traffic asynchronously after the primary route response is determined.

#### Scenario: Route has a shadow target
- **WHEN** a request matches a route configured with `shadow_target`
- **THEN** the primary target response is returned to the client
- **AND** a duplicate payload plus primary response metadata is emitted for background shadow execution

### Requirement: Shadow diff observability
Shadow execution SHALL emit divergence metrics through the observability pipeline.

#### Scenario: Shadow response differs
- **WHEN** the shadow target output differs from the primary output
- **THEN** the diff is recorded for operators without altering the client response
