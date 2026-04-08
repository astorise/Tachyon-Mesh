# Specifications: gRPC & HTTP/2 Architecture

## 1. WIT Interface Updates (`wit/tachyon.wit`)
Standard HTTP/1.1 only has headers. gRPC needs trailers.

    package tachyon:mesh;

    record http-response {
        status: u16,
        headers: list<tuple<string, string>>,
        body: list<u8>,
        // NEW: Trailers sent after the body stream
        trailers: option<list<tuple<string, string>>>,
    }

## 2. Host HTTP/2 Configuration (`core-host`)
To accept raw gRPC from internal microservices without TLS (Mesh IPC), we must enable `h2c`.
- In `main.rs`, when building the Axum server without mTLS:
  Use `hyper_util::server::conn::auto::Builder::new()` which automatically detects if the incoming byte stream is HTTP/1.1 or HTTP/2.

## 3. The Transcoding System FaaS (`system-faas-grpc-web`)
Many frontends cannot speak native HTTP/2 gRPC. They send HTTP/1.1 with base64-encoded Protobufs (gRPC-Web) or pure JSON.
- **Workflow:** 1. The middleware intercepts the request.
  2. If `Content-Type: application/grpc-web`, it decodes the base64 payload into raw binary Protobuf.
  3. It strips the `-web` from the Content-Type.
  4. It invokes the target FaaS (which is a pure gRPC WASM module).
  5. It intercepts the response and the Trailers, encodes them back into the HTTP/1.1 body format required by gRPC-Web, and returns it to the client.