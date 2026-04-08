# Tasks: Change 029 Implementation

**Agent Instruction:** Read the `proposal.md` and `specs.md`. Implement the host-provided TLS capability and the gateway System FaaS.

## [TASK-1] Update WIT and Core Host (TLS Capability)
1. Add the `tls-crypto` interface to `tachyon.wit`.
2. In `core-host/Cargo.toml`, add the `mtls` feature and `tokio-rustls`.
3. In `core-host`, implement the WIT trait. You will need to extract the raw TCP stream from the WASI context using the provided descriptor ID.
4. Perform the `TlsAcceptor::accept().await`. Store the resulting stream in the host's `ResourceTable` to allow the guest to read/write to it via handles.

## [TASK-2] Create the System Gateway FaaS
1. Create a new WASM component project `system-faas-gateway`.
2. Generate the bindings for `wasi:sockets` and `tachyon:crypto/tls`.
3. Write an infinite loop that accepts incoming TCP connections.
4. Pass each connection to `upgrade_to_mtls`.
5. Write a simple HTTP parser to read the first line (e.g., `GET /api/v1 HTTP/1.1`) from the decrypted stream. Use your internal IPC outbound capability to forward this to the requested route, then stream the response back.

## Validation Step
1. Generate test certificates (`ca.crt`, `server.crt`, `server.key`, `client.crt`, `client.key`). Provide the server certs to the `core-host` via ENV variables.
2. Add `system-faas-gateway` to the `integrity.lock` manifest.
3. Start the host. It should execute the gateway, which binds to 8443.
4. Make a `curl -k` request without a client certificate. The Host's native Rustls will reject the handshake, and the FaaS will close the connection.
5. Make a `curl` request with the client certificate. The FaaS will receive the decrypted HTTP request and successfully route it to the Mesh.