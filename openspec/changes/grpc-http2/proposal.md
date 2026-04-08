# Proposal: Change 030 - Native HTTP/2 Trailers & gRPC Middleware

## Context
To support gRPC natively without introducing extreme CPU overhead, the `core-host` must handle the HTTP/2 framing and multiplexing natively via `hyper`/`axum`. However, gRPC relies on HTTP "Trailers" (headers sent *after* the body) to communicate success or failure status codes. Our current WIT interface only supports standard HTTP/1 headers. We need to expand our HTTP capability to support Trailers, enabling developers to write standard FaaS modules that speak gRPC, as well as deploy a System FaaS to handle gRPC-Web transcoding.

## Objective
1. Upgrade the `core-host` Axum configuration to explicitly support `h2c` (HTTP/2 Cleartext) for internal Mesh routing, and HTTP/2 over TLS for external traffic.
2. Update the `tachyon.wit` HTTP interface to allow WASM guests to read and write HTTP Trailers.
3. Build a `system-faas-grpc-web` middleware that intercepts HTTP/1.1 JSON requests from frontend browsers, transcodes them to Protobuf, and forwards them as HTTP/2 gRPC to the backend FaaS.

## Scope
- Update `core-host` to read `Trailers` from the Axum `Body` and pass them to the WASI environment.
- Modify `wit/tachyon.wit` to include `get-trailers` and `set-trailers` in the HTTP response structure.
- Create the System FaaS for transcoding, proving that protocol translation can be handled purely in WASM.

## Success Metrics
- A standard gRPC client (e.g., Postman or a Go microservice) can send a Protobuf payload over HTTP/2 to a FaaS. The FaaS successfully reads it and returns a `grpc-status: 0` Trailer.
- The `core-host` remains lightweight, leveraging `hyper`'s native HTTP/2 C-level optimizations without decoding frames in WASM.