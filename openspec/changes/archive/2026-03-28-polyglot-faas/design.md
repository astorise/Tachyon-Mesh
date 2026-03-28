# Design: Polyglot WASI Guests

## Summary
`polyglot-faas` adds Go and JavaScript guest examples without introducing language-specific branches in the host router. The host already resolves a sealed route to a guest module name, so the main runtime change is entrypoint compatibility: Rust guests keep exporting `faas_entry`, while TinyGo and Javy guests are invoked through the standard WASI `_start` export.

## Build Strategy
- Add `guest-go` as a small TinyGo module that drains stdin and writes a static response.
- Add `guest-js` as a Javy script that uses `Javy.IO` to drain stdin and write a static response.
- Compile both artifacts in the container builder stage into `/workspace/guest-modules`.
- Copy those artifacts into `/app/guest-modules` in the runtime image, alongside the Rust-produced guest modules.

## Host Execution
The Wasmtime execution path remains shared across guest languages. `core-host` attempts to invoke `faas_entry` first for existing Rust guests, then falls back to `_start` for command-style WASI modules such as TinyGo and Javy outputs.

## Integrity and Verification
The sealed `integrity.lock` manifest now includes `/api/guest-go` and `/api/guest-js`. The k3d integration workflow verifies both routes after deployment to confirm the host stays language-agnostic at runtime.
