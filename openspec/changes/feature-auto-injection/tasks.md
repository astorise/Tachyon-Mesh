## 1. inject_feature_routes implementation

- [x] 1.1 Add `inject_feature_routes(config: IntegrityConfig) -> IntegrityConfig` in `core-host/src/host_core/integrity_config.rs`
- [x] 1.2 Add `#[cfg(feature = "ai-inference")]` block injecting `/system/model-broker`, `/system/ai-list-model`, `/system/ai-openai-adapter`
- [x] 1.3 Add `#[cfg(feature = "s3-persistence")]` block injecting `/system/s3-proxy`, `/system/storage-broker`
- [x] 1.4 Guard each injection with `if !config.routes.iter().any(|r| r.path == path)` to ensure idempotency
- [x] 1.5 Use `IntegrityRoute { role: RouteRole::System, version: "1.0.0".to_owned(), ..Default::default() }` for each injected route

## 2. Call sites

- [x] 2.1 Wrap `verify_integrity()?` with `inject_feature_routes(...)` in `serve_host` in `entrypoint.rs`
- [x] 2.2 Wrap `load_integrity_config_from_manifest_path_with_trusted(...)` result with `inject_feature_routes` in `reload_runtime_from_disk` in `supervisors.rs`

## 3. Version bump

- [x] 3.1 Replace all `version = "1.1.0-alpha"` with `version = "1.0.0"` in all 64 workspace `Cargo.toml` files
- [x] 3.2 Replace all `version = "1.1.0-alpha"` with `version = "1.0.0"` in `systems/manifest.toml`
- [x] 3.3 Update version string in `inject_feature_routes` to `"1.0.0"`
