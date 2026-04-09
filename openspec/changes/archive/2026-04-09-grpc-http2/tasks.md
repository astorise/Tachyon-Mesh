# Tasks: Change 030 Implementation

**Agent Instruction:** Read the `proposal.md` and `specs.md`. Implement HTTP/2 Trailer support in the host to enable native gRPC.

## [TASK-1] Enable HTTP/2 in Axum
- [x] Ensure your `axum::serve` or `hyper` builder is configured to accept HTTP/2 (it usually is by default in recent Axum versions, but explicit `http2_only` or auto-detection might be required for `h2c` internal IPC).

## [TASK-2] Implement Trailers in WIT and Host
- [x] Update `tachyon.wit` to add the `trailers` field to your HTTP response/request structures.
- [x] In `core-host/src/main.rs`, when converting the guest's HTTP response back to Axum, check if `trailers` are present.
- [x] If they are, you cannot return a simple full body. You must construct an `http_body::Body` stream that yields the data chunks, and then yields the `HeaderMap` containing the trailers at the end of the stream.

## [TASK-3] Create a basic gRPC Guest FaaS
- [x] Create `guest-grpc`. Use the `prost` crate to define a simple Protobuf message (e.g., `HelloRequest`).
- [x] Read the binary body, decode it with `prost`.
- [x] Create a `HelloResponse`, encode it to binary.
- [x] Return an `http-response` with `status: 200`, `Content-Type: application/grpc`, the binary body, and crucially, `trailers: [("grpc-status", "0")]`.

## Validation Step
- [x] Deploy `guest-grpc` via `integrity.lock`.
- [x] Use a tool like `grpcurl` or a custom client script to call the endpoint over HTTP/2.
- [x] Verify the client successfully decodes the Protobuf response and recognizes the `grpc-status: 0` OK message.
