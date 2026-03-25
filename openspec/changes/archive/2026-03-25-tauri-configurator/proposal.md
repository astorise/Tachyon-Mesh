## Why

The current `cli-signer` proves the integrity flow, but it is too narrow for the GitOps-oriented developer experience Tachyon Mesh needs. We want a single Tauri-backed tool that can run headlessly as a CLI today and evolve into a richer configurator later without replacing the underlying manifest-generation path again.

## What Changes

- Add a new `tauri-configurator` capability describing the Tauri-backed manifest generator and its headless CLI mode.
- Modify the existing `cryptographic-integrity` capability so `tachyon-cli` becomes the supported manifest-generation interface instead of `cli-signer`.
- Require a `generate` command that accepts route and memory inputs, produces a compatible `integrity.lock`, and exits without opening a desktop window when invoked from the terminal.
- Plan the migration so the legacy `cli-signer` can be removed once `tachyon-cli` is in place.

## Capabilities

### New Capabilities

- `tauri-configurator`: Tauri-backed CLI entrypoint for generating signed Tachyon Mesh configuration manifests.

### Modified Capabilities

- `cryptographic-integrity`: Replace the legacy `cli-signer` interface with `tachyon-cli` while preserving `integrity.lock` compatibility for `core-host`.

## Impact

- Adds a new `tachyon-cli` workspace member and Tauri configuration.
- Introduces a headless CLI execution path on top of the manifest-generation backend.
- Enables removal of the older `cli-signer` crate once the new tool produces compatible manifests.
