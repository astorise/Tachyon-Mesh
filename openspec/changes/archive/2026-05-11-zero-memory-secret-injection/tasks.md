# Tasks: Change 078 Implementation

**CRITICAL AGENT INSTRUCTION:** Do not archive this change without modifying the Rust codebase. You must physically write the interceptor logic. Output the modified WASI HTTP handler code to the console to prove implementation. Use 4-space indentation for injected code.

## Tasks

- [x] Internal Secret Registry
  - Open `core-host/src/store/mod.rs` (or create `core-host/src/store/secrets.rs`).
  - Implement the `SecretRegistry` structure as defined in `specs.md`.
  - Implement a function:
    `pub fn resolve_secret(placeholder: &str, target_host: &str) -> Result<String, String>`
    This function must extract the UUID, fetch the real secret, verify the `target_host` against `allowed_hosts`, and return the plaintext.

- [x] WASI HTTP Interceptor Middleware
  - Locate your WASI HTTP implementation in `core-host/src/host_core/guest_runtime.rs` (or where the `wasi_http::WasiHttpCtx` and outbound requests are handled).
  - Inside the logic that converts the WASI Request to the actual Hyper/Reqwest Request, iterate over the headers.
  - If a header value contains `tachyon:secret:`, invoke `resolve_secret`.
  - Replace the header value with the resolved plaintext.

- [x] Integration Test / Honeypot
  - Ensure that if `resolve_secret` fails (e.g., wrong host), the original placeholder is kept in the header so the external attacker only receives the useless UUID.

## Validation Step
1. Run `cargo check -p core-host`. 
2. Print the implementation of the WASI HTTP interceptor loop to the console.
