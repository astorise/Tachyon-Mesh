## Why

Operators need a one-click path to deploy pre-compiled WASM modules and their
route definitions onto a live node. Without it, each module requires a manual
asset upload + manifest edit cycle that is error-prone and blocks rapid iteration.

## What Changes

- **`tachyon-client`**: new `import_faas_package(path)` / `import_faas_package_bytes(data)` that reads a `.tar.gz`, uploads every `.wasm` file as a content-addressed asset (`tachyon://sha256:…`), and patches the live manifest with the routes declared in the embedded `manifest.json`.
- **`tachyon-mcp`**: new `tachyon_import_package` tool that exposes the above function to MCP clients.
- **`tachyon-ui`**: new Tauri command `import_faas_package(bytes)` and an *Import & Deploy* section in `TachyonWorkloadsPanel` with a file picker and deploy button.
- **`examples/guest-examples/manifest.json`**: 9-route manifest shipped inside the `guest-examples.tar.gz` CI artifact, covering all non-test guest WASMs.
- **Bug — manifest format mismatch**: `GET /admin/manifest` returns `IntegrityConfig` directly; client functions were incorrectly expecting the `{config_payload, public_key, signature}` wrapper format. Fixed in `get_manifest_config`, `load_live_config_payload`, and `get_active_config`.
- **Bug — config_version not incremented**: `patch_and_apply_manifest` was re-signing the fetched config with the same `config_version`, triggering a 409 Conflict on the node. Now increments before signing.
- **Bug — tachyon:// URIs rejected by validation**: `normalize_route_target` blocked any module name containing `/` as a filesystem path. Asset URIs (`tachyon://sha256:…`) are now exempted.
- **Bug — lockfile fallback when not connected**: `import_faas_package_bytes` now fails immediately with "not connected to a node" instead of silently falling back to a non-existent `integrity.lock`.

## Capabilities

### New Capabilities

- `faas-package-import`: Upload a `.tar.gz` FaaS package, register content-addressed WASM assets, and activate route definitions from the embedded manifest in one operation.

### Modified Capabilities

- `tauri-configurator`: New `import_faas_package` Tauri command; UI section in Workloads panel.
- `cryptographic-integrity`: `patch_and_apply_manifest` must increment `config_version`; `normalize_route_target` must accept `tachyon://` URIs; `GET /admin/manifest` client parsing corrected.
- `local-asset-registry-and-air-gapped-deployments`: Content-addressed asset upload (`push_asset_bytes`) is now the backing store for imported WASM modules.

## Impact

- **`tachyon-client/src/lib.rs`**: ~150 lines added (`import_faas_package*`, `ImportPackageResult`, `ImportedModule`); three existing functions fixed.
- **`tachyon-mcp/src/main.rs`**: tool spec + dispatch case.
- **`tachyon-ui/src/main.rs`**: Tauri command registration.
- **`tachyon-ui/src/components/domains/TachyonWorkloadsPanel.ts`**: Import section, file handler, `importPackage()` method.
- **`examples/guest-examples/manifest.json`**: new file.
- **`.github/workflows/ci.yml`**: pack step includes `manifest.json`.
- **Deps**: `flate2` + `tar` crates (already in `tachyon-client`).
