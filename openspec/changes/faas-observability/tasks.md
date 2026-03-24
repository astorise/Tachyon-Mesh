## 1. Proc-Macro SDK

- [ ] 1.1 Create a `faas-sdk` proc-macro crate in the workspace with `syn`, `quote`, and `proc-macro2`.
- [ ] 1.2 Implement `#[faas_handler]` so it injects JSON `tracing_subscriber` initialization to `stdout` before the guest handler body executes.
- [ ] 1.3 Ensure the generated entrypoint remains callable as the WASI guest function export.

## 2. Guest Instrumentation

- [ ] 2.1 Update `guest-example` dependencies to include `faas-sdk`, `tracing`, and JSON-capable `tracing-subscriber`.
- [ ] 2.2 Annotate the guest entrypoint with `#[faas_sdk::faas_handler]` and emit at least one `tracing::info!` event during request handling.
- [ ] 2.3 Preserve the guest's plain response payload on `stdout` after logging.

## 3. Host Log Interception

- [ ] 3.1 Ensure `core-host` initializes tracing and can parse guest output as JSON with `serde_json`.
- [ ] 3.2 Read the captured `MemoryWritePipe`, parse line-delimited JSON log records, and forward recognized guest logs through the host logger.
- [ ] 3.3 Return only the non-log output as the HTTP response payload.

## 4. Validation

- [ ] 4.1 Build the guest for WASI and run `core-host`.
- [ ] 4.2 Send a request to `/api/guest-example` and verify the host terminal prints forwarded guest logs.
- [ ] 4.3 Confirm the HTTP client receives the clean response body without the JSON log lines mixed in.
