# Specifications: The Realignment

## 1. Storage FaaS Isolation
- The files `core-host/src/asset_registry.rs` and `core-host/src/model_broker.rs` MUST be deleted.
- Two new System FaaS crates must be created: `systems/system-faas-registry` (for WASM blobs) and `systems/system-faas-model-broker` (for D2D chunked LLM streams).

## 2. Zero-Trust Gateway (mTLS & Auth)
- `tachyon-ui/index.html` MUST contain the `<div id="connection-overlay">` form before the closing `</body>` tag.
- `core-host/src/server_h3.rs` MUST extract the `Authorization: Bearer` token from requests and validate it using the `system-faas-auth` component before returning any administrative data.

## 3. Chunked Streaming Protocol
- The `system-faas-model-broker` MUST implement the multipart upload protocol (`init`, `upload chunk`, `commit`) using direct disk appends (`tokio::fs::OpenOptions::new().append(true)`) to prevent RAM exhaustion.