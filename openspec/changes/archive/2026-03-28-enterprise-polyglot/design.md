# Design: Enterprise Polyglot WASI Guests

## Summary
`enterprise-polyglot` extends the existing `polyglot-faas` capability with managed-language guest examples for C# and Java. The host stays language-agnostic: routes still resolve to guest module names, and the same Wasmtime/WASI execution path runs Rust, Go, JavaScript, C#, and Java modules without per-language branches in production code.

## Managed Guest Build Strategy
- Add `guest-csharp` as a .NET 8 console project targeting `wasi-wasm`.
- Publish the C# guest as a single-file WASI module so the host can execute `guest_csharp.wasm` directly, without a sidecar runtime bundle.
- Add `guest-java` as a Maven project that uses `teavm-maven-plugin` with `WEBASSEMBLY_WASI` output.
- Emit both managed artifacts into `/workspace/guest-modules` during the container build, alongside the existing TinyGo and Javy outputs.

## Host Compatibility
The current host already derives a guest module name from the sealed route path and normalizes `-` to `_` when resolving `*.wasm` files. That behavior is sufficient for `/api/guest-csharp` and `/api/guest-java`, so the change only needs regression tests that lock in the hyphenated route-to-module mapping relied on by the new guests.

Managed-language modules have a higher startup cost than the existing Rust, Go, and JavaScript guests. To keep execution bounded while allowing these modules to boot, the sealed configuration raises the default guest fuel budget. The host remains quota-driven, but with a ceiling sized for managed runtimes instead of tiny native guests only.

## Packaging and Verification
The root `Dockerfile` remains the single build entrypoint. It now installs the .NET 8 WASI workload, `wasi-sdk`, Maven, and OpenJDK 17 in the builder stage, publishes `guest_csharp.wasm`, compiles `guest_java.wasm`, copies both into the runtime image, and regenerates `integrity.lock` with the new sealed routes.

The integration workflow verifies that the deployed host serves all four non-Rust polyglot routes:
- `/api/guest-go`
- `/api/guest-js`
- `/api/guest-csharp`
- `/api/guest-java`
