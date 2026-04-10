# Specifications: Host-Terminated mTLS Gateway

## 1. System Guest World Updates (`wit/tachyon.wit`)
The system gateway forwards accepted requests through the existing mesh transport.

- Extend `world system-faas-guest` with `import outbound-http;`.
- Keep the gateway itself as a normal system handler component instead of a long-lived raw socket owner.

## 2. Host mTLS Listener (`core-host`)
The host owns the TLS handshake and enforces client authentication before any request reaches the gateway logic.

- Add an `mtls` feature to `core-host`.
- Load `TACHYON_MTLS_SERVER_CERT_PEM`, `TACHYON_MTLS_SERVER_KEY_PEM`, `TACHYON_MTLS_CA_CERT_PEM`, and optional `TACHYON_MTLS_ADDRESS`.
- Build a Rustls `ServerConfig` with `WebPkiClientVerifier`.
- Start the listener only if `/system/gateway` is present in the sealed manifest.
- After a successful handshake, route the request to `/system/gateway` and preserve the original URI in `x-tachyon-original-route`.

## 3. The Gateway FaaS (`system-faas-gateway`)
The gateway is a standard system component that forwards already-authenticated requests into the mesh.

- Read `x-tachyon-original-route`.
- Reject empty routes and self-forwarding to `/system/gateway`.
- Strip hop-by-hop and internal headers.
- Call `tachyon::mesh::outbound-http::send-request("http://mesh{route}", ...)`.

## 4. Validation Shape
This change is validated without external PKI.

- Generate CA, server, and client certificates in tests with `rcgen`.
- Verify that a request without a client certificate fails during the handshake.
- Verify that an authorized request is forwarded to a sealed target route through `/system/gateway`.
