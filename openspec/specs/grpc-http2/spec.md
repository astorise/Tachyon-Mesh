# grpc-http2 Specification

## Purpose
Define native gRPC and HTTP/2 support for FaaS routes, including frame handling, trailer mapping, and streaming behavior.

## Requirements
### Requirement: gRPC over HTTP/2 routing
The host SHALL route gRPC requests over HTTP/2 while preserving request metadata, response trailers, and status semantics.

#### Scenario: Unary gRPC request succeeds
- **WHEN** a sealed gRPC route receives a valid unary HTTP/2 request
- **THEN** the request is dispatched to the configured guest
- **AND** the response maps guest output and trailers into a valid gRPC response

### Requirement: Interop conformance
The CI pipeline SHALL validate gRPC behavior against the repository interop suite.

#### Scenario: Interop suite runs
- **WHEN** the gRPC interop workflow executes standard cases such as empty unary and cancellation
- **THEN** each case must pass before the change is considered releasable
