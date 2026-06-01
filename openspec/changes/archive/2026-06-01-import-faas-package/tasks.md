## 1. Core client implementation

- [x] 1.1 Add `ImportedModule` and `ImportPackageResult` structs to `tachyon-client/src/lib.rs`
- [x] 1.2 Implement `import_faas_package(path)` delegating to `import_faas_package_bytes`
- [x] 1.3 Implement `import_faas_package_bytes(data)`: extract tar.gz, upload WASMs, patch manifest
- [x] 1.4 Add fail-fast connection check at entry of `import_faas_package_bytes`
- [x] 1.5 Store module URIs under both `_` and `-` name forms for manifest resolution

## 2. Bug fixes — manifest protocol

- [x] 2.1 Fix `get_manifest_config` to deserialise `GET /admin/manifest` as `serde_json::Value` directly
- [x] 2.2 Fix `load_live_config_payload` same as above (keep lockfile fallback only for offline path)
- [x] 2.3 Fix `get_active_config` same as above
- [x] 2.4 Add `config_version` increment to `patch_and_apply_manifest` before signing
- [x] 2.5 Exempt `tachyon://` URIs from the filesystem-path check in `normalize_route_target`

## 3. MCP tool

- [x] 3.1 Add `tachyon_import_package` to `missing_required_args` and `rate_limit_spec` maps
- [x] 3.2 Add tool JSON spec (name, description, inputSchema) in `tachyon-mcp/src/main.rs`
- [x] 3.3 Add dispatch case calling `tachyon_client::import_faas_package`

## 4. Tauri command and UI

- [x] 4.1 Add `import_faas_package` Tauri command in `tachyon-ui/src/main.rs`
- [x] 4.2 Register the command in `invoke_handler`
- [x] 4.3 Add `ImportPackageResult` type and `importFile` state to `TachyonWorkloadsPanel`
- [x] 4.4 Render *Import & Deploy* section with file picker and deploy button
- [x] 4.5 Wire file-input change and button click handlers in `bindForm()`
- [x] 4.6 Implement `importPackage()` method reading `ArrayBuffer` and calling `invoke`
- [x] 4.7 Add i18n keys (`workloads.import.*`) for both `en` and `fr`

## 5. Guest examples manifest

- [x] 5.1 Create `examples/guest-examples/manifest.json` with 9 routes (all non-test guest WASMs)
- [x] 5.2 Update `.github/workflows/ci.yml` pack step to include `manifest.json` in the artifact
