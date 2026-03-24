## 1. Dependencies and Host Scaffolding

- [x] 1.1 Add the `axum` and `bytes` dependencies required to expose the host over HTTP.
- [x] 1.2 Rewrite `core-host/src/main.rs` so the host starts an `axum::Router` listening on `0.0.0.0:8080`.
- [x] 1.3 Implement a catch-all handler that resolves the requested function name from the URL path.

## 2. Guest Standard I/O Contract

- [x] 2.1 Update `guest-example` so it reads the full request payload from `stdin`.
- [x] 2.2 Transform the input into a response string and write that response to `stdout`.

## 3. Request-Scoped WASI Pipes

- [x] 3.1 Create a `MemoryReadPipe` from the incoming HTTP body for each request.
- [x] 3.2 Attach a fresh `MemoryWritePipe` to capture guest output for the same request.
- [x] 3.3 Build a per-request `WasiCtx`, instantiate the module, invoke the guest entrypoint, and return the captured stdout bytes as the HTTP response.

## 4. Validation

- [x] 4.1 Build the guest for WASI and run `core-host`.
- [x] 4.2 Send a `POST` request to `/api/guest-example` with a sample payload.
- [x] 4.3 Confirm the HTTP response matches the output written by the guest through virtual stdout.
