## 1. Polyglot Guest Sources

- [x] 1.1 Add `guest-go/go.mod` and `guest-go/main.go` so TinyGo can build a WASI guest that drains stdin and writes `Hello from TinyGo FaaS!`.
- [x] 1.2 Add `guest-js/index.js` using the Javy `Javy.IO` API to drain stdin and write `Hello from JavaScript FaaS!`.

## 2. Host Runtime Compatibility

- [x] 2.1 Update `core-host` to invoke `faas_entry` for existing Rust guests and fall back to `_start` for command-style WASI guests.

## 3. Packaging and Sealed Routes

- [x] 3.1 Extend the `Dockerfile` builder stage to install Go, TinyGo, and Javy, compile the Go and JavaScript guests into `/workspace/guest-modules`, and copy them into the runtime image.
- [x] 3.2 Regenerate `integrity.lock` so `/api/guest-go` and `/api/guest-js` are sealed alongside the existing routes.

## 4. Verification

- [x] 4.1 Extend `.github/workflows/integration.yml` to assert the deployed responses for `/api/guest-go` and `/api/guest-js`.
- [x] 4.2 Verify the change with `cargo test --workspace` and `openspec validate --all`.
