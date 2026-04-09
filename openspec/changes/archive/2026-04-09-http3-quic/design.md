# Design: HTTP/3 over QUIC

## Summary

Add an HTTP/3 listener behind the `http3` Cargo feature in `core-host`.
The listener binds a QUIC endpoint on the HTTPS address, reuses the existing
TLS configuration path, and forwards accepted requests into the same Axum
routing pipeline used by the HTTP/1.1 and HTTP/2 stack.

## Architecture

The implementation introduces a dedicated `server_h3` module compiled only when
the `http3` feature is enabled. At host startup, the runtime attempts to create
an HTTP/3 listener after the HTTPS listener is initialized. The listener:

- builds a `quinn::Endpoint` from the provisioned `rustls::ServerConfig`
- accepts QUIC connections asynchronously
- wraps each connection with `h3-quinn`
- converts each HTTP/3 request into a standard `http::Request`
- dispatches the request through the existing Axum `Router`
- serializes the resulting response back over the HTTP/3 stream

## TLS and ALPN

HTTP/3 requires QUIC-compatible TLS settings and the `h3` ALPN identifier. The
listener therefore derives a QUIC server configuration from the existing TLS
manager and sets ALPN to `h3` while preserving the current certificate
provisioning behavior.

## Feature Gating

The new dependency surface and listener startup path are both hidden behind the
`http3` Cargo feature. Builds without that feature continue to run the existing
HTTP/TLS stack without binding a QUIC socket.

## Validation

Validation is provided by a dedicated feature-gated integration test that
starts the host listener, establishes a QUIC client connection, issues an
HTTP/3 request to a guest-backed route, and verifies that the Axum/FaaS
pipeline produces the expected response body.
