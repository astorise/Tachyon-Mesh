## 1. Malicious Guest Fixture

- [ ] 1.1 Add a `guest-malicious` WASI crate to the workspace as a `cdylib`.
- [ ] 1.2 Export guest behavior that intentionally loops forever or attempts excessive allocation to trigger quota enforcement.

## 2. Engine and Store Quotas

- [ ] 2.1 Configure the shared Wasmtime engine with fuel consumption enabled.
- [ ] 2.2 Inject a bounded fuel budget into each request-scoped store before invoking the guest.
- [ ] 2.3 Enforce a 50 MiB guest memory ceiling using the most idiomatic limit mechanism supported by the current Wasmtime version.

## 3. Trap Handling

- [ ] 3.1 Update the host execution path so guest traps are handled as recoverable request failures.
- [ ] 3.2 Log a warning when a guest exceeds fuel or memory limits.
- [ ] 3.3 Return HTTP `500 Internal Server Error` with `Execution trapped: Resource limit exceeded` while keeping the host process alive.

## 4. Validation

- [ ] 4.1 Build both the normal guest and `guest-malicious` for WASI, then run `core-host`.
- [ ] 4.2 Confirm a normal guest request still succeeds with HTTP 200.
- [ ] 4.3 Confirm a malicious guest request is trapped quickly with HTTP 500 and does not crash the host.
