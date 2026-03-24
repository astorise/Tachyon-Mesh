# Proposal: Change 002 - HTTP Routing & FaaS I/O

## Context
In Change 001, we successfully built a core engine capable of executing a WASM module. However, a FaaS platform must respond to network events. We need to expose our WASM engine via an HTTP server without relying on external sidecars like Envoy or Knative Queue-Proxy.

## Objective
Integrate an HTTP server (`axum`) into the Rust Host. The host must receive HTTP requests, map the URL to a specific WASM module, pass the request payload into the WASM module's standard input (stdin), and capture its standard output (stdout) to return as the HTTP response.

## Scope
- Integrate `axum` as the web framework in `core-host`.
- Implement a wildcard route handler (`/*path`).
- Use in-memory WASI pipes (`MemoryReadPipe` and `MemoryWritePipe`) to stream data between the Axum HTTP context and the WASM isolated memory.
- Update `guest-example` to read from stdin and write to stdout.

## Out of Scope
- Complex HTTP Headers passing (we will stick to the body payload for now).
- The Component Model (WIT) / Advanced bindings.
- Cryptographic checks.

## Success Metrics
- Sending a `POST` request with a JSON body via `curl` triggers the WASM module.
- The WASM module successfully reads the body, processes it, and prints a response.
- The Host captures the output and sends it back to the `curl` client with an HTTP 200 status.