# Tasks: Change 029 Implementation

**Agent Instruction:** Implement the host-terminated mTLS gateway architecture and keep the OpenSpec artifacts aligned with the shipped design.

## [TASK-1] Add host-managed mTLS gateway support
- [x] Add an `mtls` feature to `core-host` and keep the Rustls-based gateway listener available behind explicit runtime configuration.
- [x] Load server certificate, private key, client CA, and bind address from environment variables.
- [x] Build a Rustls server config that requires verified client certificates and supports HTTP/1.1 plus HTTP/2.
- [x] Start a dedicated mTLS listener only when the sealed manifest defines `/system/gateway`.
- [x] Rewrite authorized mTLS requests to `/system/gateway` and preserve the original path in `x-tachyon-original-route`.

## [TASK-2] Create the System Gateway FaaS
- [x] Create the `system-faas-gateway` WASM component project.
- [x] Expose `outbound-http` to the `system-faas-guest` world so the gateway can forward requests through the mesh.
- [x] Implement the gateway handler so it validates `x-tachyon-original-route`, rejects self-forwarding, strips internal headers, and proxies the request to `http://mesh/...`.
- [x] Add unit tests for route normalization and forwarded header filtering.

## Validation Step
- [x] Generate mock CA, server, and client certificates in tests.
- [x] Build the `system-faas-gateway` WASM artifact locally and in CI.
- [x] Validate that a request without a client certificate fails the mTLS handshake.
- [x] Validate that a request with a trusted client certificate is forwarded through `/system/gateway` to the sealed mesh route.
- [x] Document the deployment shape in the main `mtls-encryption` spec and archive the completed change.
