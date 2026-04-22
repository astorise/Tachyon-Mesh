# Design: Local Asset Registry and Air-Gapped Deployments

## Storage Model
- The embedded `CoreStore` now includes an `AssetRegistry` bucket keyed by `sha256:<digest>`.
- Uploaded binaries are also materialized into an `asset-registry/` directory next to the manifest so the existing module loader can resolve them uniformly.

## Upload Surface
- `POST /admin/assets` accepts raw `.wasm` bytes behind the admin middleware.
- The client calculates the SHA-256 checksum locally and sends it in `x-tachyon-expected-sha256`.
- The host persists the blob, materializes it locally, and returns the canonical `tachyon://sha256:...` URI.

## Loader Integration
- `resolve_guest_module_path` now detects `tachyon://sha256:...`.
- The URI is resolved through the embedded asset store and written into the materialized registry directory before Wasmtime opens the module.

## UI Surface
- The desktop dashboard exposes an "Air-Gapped Asset Registry" panel for `.wasm` uploads.
- The Tauri command accepts file metadata plus bytes so the browser layer can upload selected files without requiring direct filesystem paths from the webview.
