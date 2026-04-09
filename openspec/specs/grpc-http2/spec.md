# gRPC HTTP/2

## Purpose
Define how Tachyon preserves HTTP/2 and trailer semantics required by gRPC, exposes request and response metadata to Wasm guests, and provides a reference guest that answers protobuf RPC traffic over h2c.
## Requirements
### Requirement: The HTTP contract preserves gRPC-oriented metadata
The Tachyon WIT package SHALL expose request headers, request trailers, response headers, and response trailers to component guests so a guest can participate in gRPC exchanges without raw frame parsing in Wasm.

#### Scenario: A guest returns gRPC trailers
- **WHEN** a component guest returns an HTTP response with trailers such as `grpc-status`
- **THEN** the host preserves those trailers on the outgoing HTTP response
- **AND** the host forwards ordinary headers such as `content-type: application/grpc`

### Requirement: The cleartext HTTP listener accepts HTTP/2 traffic
The host SHALL accept HTTP/2 cleartext traffic on the primary HTTP listener so internal or local gRPC clients can use h2c without native TLS.

#### Scenario: A gRPC client connects over h2c
- **WHEN** a client opens an HTTP/2 cleartext connection to a sealed Tachyon route
- **THEN** the host accepts the connection
- **AND** dispatches the request through the standard route execution path

### Requirement: The workspace ships a reference gRPC guest
The workspace SHALL include a guest component that decodes a protobuf request, encodes a protobuf response, and returns a successful `grpc-status` trailer through the shared HTTP contract.

#### Scenario: The sample guest answers a unary RPC
- **WHEN** a client sends a framed protobuf request to the sealed gRPC route
- **THEN** the guest decodes the protobuf payload
- **AND** returns a framed protobuf response body
- **AND** the client observes `grpc-status: 0` in the HTTP/2 trailers

### Requirement: The HTTP contract supports HTTP/2 trailers and gRPC-oriented middleware
The host SHALL preserve HTTP/2 semantics required by gRPC, including trailers, and SHALL allow middleware to translate browser-facing protocols into backend gRPC requests.

#### Scenario: A route handles gRPC traffic
- **WHEN** a request path requires HTTP/2 trailers or gRPC-Web translation
- **THEN** the host preserves trailers and stream semantics
- **AND** middleware can transcode frontend traffic into backend gRPC exchanges

