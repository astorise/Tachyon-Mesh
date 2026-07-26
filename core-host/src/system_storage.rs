use anyhow::{anyhow, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use wasmtime::{component::Linker as ComponentLinker, Engine, Store};
use wasmtime_wasi::{
    DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

const ASSET_URI_PREFIX: &str = "tachyon://sha256:";
const REGISTRY_MODULE_NAME: &str = "system-faas-registry";
const MODEL_BROKER_MODULE_NAME: &str = "system-faas-model-broker";

mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "system-faas-guest",
    });
}

struct StorageComponentState {
    ctx: WasiCtx,
    table: ResourceTable,
    core_store: Arc<crate::store::CoreStore>,
    #[cfg(feature = "s3-persistence")]
    s3_backend: Option<Arc<crate::persistence::S3PersistenceBackend>>,
    #[cfg(feature = "s3-persistence")]
    root_dir: PathBuf,
    #[cfg(feature = "s3-persistence")]
    core_store_path: PathBuf,
}

const AI_MODELS_REGISTRY_TABLE: &str = "ai-models-registry";

struct ComponentRequest {
    method: String,
    uri: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

struct ComponentResponse {
    status: StatusCode,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    trailers: Vec<(String, String)>,
}

pub(crate) fn asset_registry_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("asset-registry")
}

fn model_broker_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("tachyon_data")
}

/// Host-side directory where `system-faas-model-broker` unpacks uploaded models
/// (`{tachyon_data}/models/{alias}/`). The broker's preopened `.` maps to
/// `model_broker_dir`, so this is exactly where it writes; the AI runtime reads
/// the same path to lazily load uploaded checkpoints.
#[cfg(feature = "ai-inference")]
pub(crate) fn model_broker_models_dir(manifest_path: &Path) -> PathBuf {
    model_broker_dir(manifest_path).join("models")
}

pub(crate) fn is_asset_uri(value: &str) -> bool {
    value.starts_with(ASSET_URI_PREFIX)
}

pub(crate) fn resolve_asset_uri(manifest_path: &Path, uri: &str) -> Result<PathBuf> {
    let hash = hash_from_asset_uri(uri)?;
    let path = asset_registry_dir(manifest_path)
        .join("assets")
        .join(format!("{}.wasm", hash.trim_start_matches("sha256:")));
    if !path.exists() {
        anyhow::bail!("asset `{uri}` was not found in the embedded registry");
    }
    Ok(path.canonicalize().unwrap_or(path))
}

// Reachable only via `admin_plane::authenticated_routes` (`/admin/assets`,
// `/admin/models/*`), gated behind `admin-plane`. `allow(dead_code)` avoids
// chasing this through the shared `proxy_request_to_component` plumbing below.
#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn upload_asset_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, REGISTRY_MODULE_NAME, asset_registry_dir).await
}

#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn init_upload_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, MODEL_BROKER_MODULE_NAME, model_broker_dir).await
}

#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn upload_chunk_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, MODEL_BROKER_MODULE_NAME, model_broker_dir).await
}

#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn commit_upload_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, MODEL_BROKER_MODULE_NAME, model_broker_dir).await
}

async fn proxy_request_to_component(
    state: crate::AppState,
    request: Request,
    module_name: &'static str,
    working_dir: fn(&Path) -> PathBuf,
) -> Response {
    let manifest_path = state.manifest_path.clone();
    let runtime = state.runtime.load();
    let engine = runtime.engine.clone();
    let component_cache = Arc::clone(&runtime.component_cache);
    drop(runtime);
    let core_store = state.core_store.clone();
    #[cfg(feature = "s3-persistence")]
    let s3_backend = state.s3_backend.clone();
    #[cfg(feature = "s3-persistence")]
    let core_store_path = crate::host_core::core_store_path(&manifest_path);
    let component_request = match collect_component_request(request).await {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to collect admin storage request: {error}"),
            )
                .into_response();
        }
    };
    let root_dir = working_dir(&manifest_path);

    match tokio::task::spawn_blocking(move || {
        invoke_storage_component(
            &engine,
            module_name,
            &root_dir,
            core_store,
            #[cfg(feature = "s3-persistence")]
            s3_backend,
            #[cfg(feature = "s3-persistence")]
            core_store_path,
            component_cache,
            component_request,
        )
    })
    .await
    {
        Ok(Ok(response)) => {
            if module_name == REGISTRY_MODULE_NAME && response.status.is_success() {
                let runtime = state.runtime.load();
                runtime.instance_pool.invalidate_all();
                runtime.component_cache.invalidate_all();
                runtime.component_instance_pre_cache.invalidate_all();
                runtime.legacy_instance_pre_cache.invalidate_all();
                runtime.instance_pool.run_pending_tasks();
                runtime.component_cache.run_pending_tasks();
                runtime.component_instance_pre_cache.run_pending_tasks();
                runtime.legacy_instance_pre_cache.run_pending_tasks();
            }
            component_response_to_http(response)
        }
        Ok(Err(error)) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to join admin storage task: {error}"),
        )
            .into_response(),
    }
}

async fn collect_component_request(request: Request) -> Result<ComponentRequest> {
    let (parts, body) = request.into_parts();
    let collected = body
        .collect()
        .await
        .context("failed to read proxied request body")?;
    Ok(ComponentRequest {
        method: parts.method.as_str().to_owned(),
        uri: parts.uri.to_string(),
        headers: header_map_to_fields(&parts.headers),
        body: collected.to_bytes().to_vec(),
    })
}

fn invoke_storage_component(
    engine: &Engine,
    module_name: &str,
    root_dir: &Path,
    core_store: Arc<crate::store::CoreStore>,
    #[cfg(feature = "s3-persistence")] s3_backend: Option<
        Arc<crate::persistence::S3PersistenceBackend>,
    >,
    #[cfg(feature = "s3-persistence")] core_store_path: PathBuf,
    component_cache: Arc<moka::sync::Cache<PathBuf, crate::CachedComponent>>,
    request: ComponentRequest,
) -> Result<ComponentResponse> {
    tracing::info!(
        module = module_name,
        method = %request.method,
        uri = %request.uri,
        body_bytes = request.body.len(),
        "storage component request received"
    );
    fs::create_dir_all(root_dir).with_context(|| {
        format!(
            "failed to initialize storage component root directory `{}`",
            root_dir.display()
        )
    })?;
    let module_path = crate::resolve_guest_module_path(module_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    // Large model uploads are split into many chunked requests (one per
    // `MODEL_CHUNK_BYTES`), each of which proxies through this function. Loading
    // through the cwasm cache means only the first request per engine pays the
    // Cranelift compile; every later chunk just deserializes the cached
    // precompiled artifact, so per-chunk host overhead stays flat instead of
    // growing with the number of chunks (and thus the model size).
    let component = crate::load_component_with_pool(
        engine,
        &module_path,
        &core_store,
        "default",
        Some(&component_cache),
    )
    .map_err(|error| {
        anyhow!(
            "failed to load storage component `{module_name}` from `{}`: {error:#}",
            module_path.display()
        )
    })?;

    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        anyhow!("failed to add WASI preview2 functions to storage component linker: {error}")
    })?;
    bindings::tachyon::mesh::model_events::add_to_linker::<_, StorageComponentState>(
        &mut linker,
        |state| state,
    )
    .map_err(|error| anyhow!("failed to add model-events functions to storage linker: {error}"))?;

    let mut builder = WasiCtxBuilder::new();
    builder
        .preopened_dir(
            root_dir,
            ".",
            DirPerms::READ | DirPerms::MUTATE,
            FilePerms::READ | FilePerms::WRITE,
        )
        .map_err(|error| {
            anyhow!(
                "failed to preopen storage component root directory `{}`: {error}",
                root_dir.display()
            )
        })?;

    let mut store = Store::new(
        engine,
        StorageComponentState {
            ctx: builder.build(),
            table: ResourceTable::new(),
            core_store,
            #[cfg(feature = "s3-persistence")]
            s3_backend,
            #[cfg(feature = "s3-persistence")]
            root_dir: root_dir.to_path_buf(),
            #[cfg(feature = "s3-persistence")]
            core_store_path,
        },
    );
    let bindings = bindings::SystemFaasGuest::instantiate(&mut store, component.as_ref(), &linker)
        .map_err(|error| anyhow!("failed to instantiate storage component: {error}"))?;
    let response = bindings
        .tachyon_mesh_handler()
        .call_handle_request(
            &mut store,
            &bindings::exports::tachyon::mesh::handler::Request {
                method: request.method,
                uri: request.uri,
                headers: request.headers,
                body: request.body,
                trailers: Vec::new(),
            },
        )
        .map_err(|error| anyhow!("storage component trapped: {error}"))?;

    tracing::info!(
        module = module_name,
        status = response.status,
        body_bytes = response.body.len(),
        "storage component request completed"
    );
    Ok(ComponentResponse {
        status: StatusCode::from_u16(response.status).map_err(|error| {
            anyhow!(
                "storage component returned an invalid HTTP status code `{}`: {error}",
                response.status
            )
        })?,
        headers: response.headers,
        body: response.body,
        trailers: response.trailers,
    })
}

// The `ai-models-registry` kv-partition table is owned by `guest-openai`, which
// reads/writes its `ModelInfo` with `#[serde(rename_all = "camelCase")]`. The
// broker writes the same table directly on upload, so it MUST use the identical
// casing — otherwise `guest-openai`'s `list_models` fails to deserialize the
// row (missing required `vramRequiredMb`) and silently drops it, so uploaded
// models never surface in `GET /ai/v1/models`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryModelInfo<'a> {
    alias: &'a str,
    engine: &'a str,
    vram_required_mb: u64,
    status: &'a str,
    model_path: &'a str,
    /// Who wrote this row. Absent on upload-published rows (they predate the
    /// field), `"config"` on manifest-derived ones. Reconciliation needs the
    /// distinction: a configured row must be refreshed or dropped when the
    /// manifest changes, while an uploaded row — which carries a real on-disk
    /// path and VRAM figure — must survive untouched. `guest-openai` ignores
    /// unknown fields, so adding it does not disturb the reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
}

/// Marks a registry row as derived from the manifest rather than an upload.
#[cfg(feature = "ai-inference")]
const REGISTRY_SOURCE_CONFIG: &str = "config";

/// Engine label recorded for a configured binding.
///
/// `guest-openai` builds each `GET /ai/v1/models` id as `{engine}/{alias}` and
/// resolves a request against either form, so this string is part of the public
/// model id — not just metadata.
#[cfg(feature = "ai-inference")]
fn binding_engine_label(path: &str) -> &'static str {
    // Classify the same trimmed value `UpstreamEndpoint::parse` accepts, or a
    // path with leading whitespace loads as a working upstream while the
    // registry advertises it as `safetensors/<alias>`.
    let path = path.trim();
    if path.starts_with(crate::ai_inference::UPSTREAM_SCHEME) {
        "openai"
    } else if path == "mock" || path.starts_with("mock:") {
        "mock"
    } else {
        // Probe the directory once and classify by what is actually in it. The
        // label is part of the public model id (`{engine}/{alias}`), so an
        // ONNX embedding directory advertised as `safetensors` gives clients
        // wrong format metadata.
        let mut has_gguf = false;
        let mut has_onnx = false;
        if let Ok(entries) = std::path::Path::new(path).read_dir() {
            for entry in entries.flatten() {
                match entry.path().extension().and_then(|ext| ext.to_str()) {
                    Some(ext) if ext.eq_ignore_ascii_case("gguf") => has_gguf = true,
                    Some(ext) if ext.eq_ignore_ascii_case("onnx") => has_onnx = true,
                    _ => {}
                }
            }
        }
        match (has_gguf, has_onnx) {
            // A GGUF file wins: a directory carrying both is a quantized
            // checkpoint that happens to ship an ONNX sidecar.
            (true, _) => "gguf",
            (false, true) => "onnx",
            (false, false) => "safetensors",
        }
    }
}

/// Publish the route-configured model bindings into the registry table.
///
/// Until now the table was written only by the upload flow, so a model declared
/// in the manifest — every `openai:` upstream, and every operator-provisioned
/// local checkpoint — was invisible in `GET /ai/v1/models`. Requests still
/// worked (`guest-openai` falls back to passing an unslashed alias straight to
/// `load_model`), but the model never appeared in a client's model picker.
///
/// Existing rows are left untouched: an upload-published entry carries a real
/// on-disk path and VRAM figure, and must win over anything derived from
/// config. `dynamic` bindings are skipped entirely — they are registered by the
/// upload that materialises them.
#[cfg(feature = "ai-inference")]
pub(crate) fn publish_configured_model_bindings(
    core_store: &crate::store::CoreStore,
    config: &crate::IntegrityConfig,
) {
    let mut configured = std::collections::HashSet::new();
    for route in &config.routes {
        for binding in &route.models {
            if binding.dynamic || binding.path.trim().is_empty() {
                continue;
            }
            configured.insert(binding.alias.as_str());
            let info = RegistryModelInfo {
                alias: &binding.alias,
                engine: binding_engine_label(&binding.path),
                // Unknown for a configured binding: the upload path is what
                // measures a checkpoint. `0` is the registry's documented
                // "unknown" value, not a claim that the model is free.
                vram_required_mb: 0,
                status: "available",
                model_path: &binding.path,
                source: Some(REGISTRY_SOURCE_CONFIG),
            };
            let Ok(value) = serde_json::to_vec(&info) else {
                continue;
            };

            // Insert-if-absent in one transaction. A read followed by a write
            // cannot express "the existing row wins": an upload committing
            // between the two would be overwritten by this zero-VRAM row.
            match core_store.kv_partition_insert_if_absent(
                AI_MODELS_REGISTRY_TABLE,
                &binding.alias,
                &value,
            ) {
                Ok(true) => {
                    tracing::info!(
                        alias = %binding.alias,
                        engine = %info.engine,
                        "published configured model binding to `{AI_MODELS_REGISTRY_TABLE}`"
                    );
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(
                        alias = %binding.alias,
                        "failed to publish configured model binding to the registry: {error:#}"
                    );
                    continue;
                }
            }

            // A row already exists. Refresh it only if this publisher owns it:
            // a hot reload that changes an alias from `openai:` to a GGUF
            // checkpoint must stop advertising `openai/<alias>`, while an
            // upload-published row keeps its real path and VRAM figure.
            //
            // Ownership is re-read inside the write transaction. Checking in a
            // separate read would let an upload land in between, and the
            // refresh would then overwrite the row the check was protecting.
            if let Err(error) = core_store.kv_partition_update(
                AI_MODELS_REGISTRY_TABLE,
                &binding.alias,
                |current| {
                    if row_is_config_owned(current) {
                        crate::store::KvPartitionUpdate::Set(value)
                    } else {
                        crate::store::KvPartitionUpdate::Keep
                    }
                },
            ) {
                tracing::warn!(
                    alias = %binding.alias,
                    "failed to refresh configured model binding in the registry: {error:#}"
                );
            }
        }
    }

    // Drop configured rows whose binding is gone, or a removed alias would stay
    // listed in `GET /ai/v1/models` forever. Upload-owned rows are never swept:
    // their model is still on disk and reachable.
    for alias in config_owned_aliases(core_store) {
        if configured.contains(alias.as_str()) {
            continue;
        }
        // Same race: an upload may have replaced this row since the scan, so
        // ownership is re-checked inside the deleting transaction.
        if let Err(error) =
            core_store.kv_partition_update(AI_MODELS_REGISTRY_TABLE, &alias, |current| {
                if row_is_config_owned(current) {
                    crate::store::KvPartitionUpdate::Delete
                } else {
                    crate::store::KvPartitionUpdate::Keep
                }
            })
        {
            tracing::warn!(
                %alias,
                "failed to drop a stale configured model binding from the registry: {error:#}"
            );
        } else {
            tracing::info!(%alias, "dropped a configured model binding that left the manifest");
        }
    }
}

/// Whether a registry row's raw bytes say this publisher wrote it. Takes the
/// value rather than the key so the caller can decide inside a transaction.
#[cfg(feature = "ai-inference")]
fn row_is_config_owned(row: Option<&[u8]>) -> bool {
    row.and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
        .and_then(|row| {
            row.get("source")
                .and_then(serde_json::Value::as_str)
                .map(|source| source == REGISTRY_SOURCE_CONFIG)
        })
        .unwrap_or(false)
}

/// Every alias in the registry whose row this publisher owns.
#[cfg(feature = "ai-inference")]
fn config_owned_aliases(core_store: &crate::store::CoreStore) -> Vec<String> {
    core_store
        .kv_partition_get_range(AI_MODELS_REGISTRY_TABLE, "", "\u{10ffff}", 10_000, 0)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(alias, raw)| {
            let row = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
            (row.get("source").and_then(serde_json::Value::as_str) == Some(REGISTRY_SOURCE_CONFIG))
                .then_some(alias)
        })
        .collect()
}

impl bindings::tachyon::mesh::model_events::Host for StorageComponentState {
    fn publish_model_uploaded(
        &mut self,
        event: bindings::tachyon::mesh::model_events::ModelUploaded,
    ) -> std::result::Result<(), String> {
        if event.alias.trim().is_empty() {
            return Err("model upload event alias must not be empty".to_owned());
        }
        if event.engine.trim().is_empty() {
            return Err("model upload event engine must not be empty".to_owned());
        }
        let info = RegistryModelInfo {
            alias: &event.alias,
            engine: &event.engine,
            vram_required_mb: 0,
            status: "available",
            model_path: &event.model_path,
            source: None,
        };
        let value = serde_json::to_vec(&info)
            .map_err(|error| format!("failed to encode model registry entry: {error}"))?;
        tracing::info!(
            alias = %event.alias,
            engine = %event.engine,
            model_path = %event.model_path,
            "publishing uploaded model to registry `{AI_MODELS_REGISTRY_TABLE}`"
        );
        self.core_store
            .kv_partition_set(AI_MODELS_REGISTRY_TABLE, &event.alias, &value)
            .map_err(|error| format!("failed to publish model upload event: {error:#}"))?;
        tracing::info!(alias = %event.alias, "model registry entry written; scheduling S3 flush");
        self.flush_uploaded_model_to_s3(&event.alias);
        Ok(())
    }
}

impl StorageComponentState {
    fn flush_uploaded_model_to_s3(&self, _alias: &str) {
        #[cfg(feature = "s3-persistence")]
        {
            let Some(backend) = self.s3_backend.clone() else {
                tracing::warn!(
                    alias = %_alias,
                    "S3 model flush skipped: no S3 backend configured (TACHYON_S3_* unset or backend init failed)"
                );
                return;
            };
            let Some(model_dir) = uploaded_model_dir(&self.root_dir, _alias) else {
                tracing::warn!(alias = %_alias, "skipping S3 model flush for invalid upload alias");
                return;
            };
            tracing::info!(
                alias = %_alias,
                model_dir = %model_dir.display(),
                "starting S3 flush of uploaded model"
            );
            let alias = _alias.to_owned();
            let core_store_path = self.core_store_path.clone();
            tokio::spawn(async move {
                if let Err(error) = backend.flush_path(&model_dir).await {
                    tracing::warn!(
                        alias = %alias,
                        path = %model_dir.display(),
                        error = %error,
                        "failed to flush uploaded model to S3"
                    );
                } else {
                    tracing::info!(
                        alias = %alias,
                        path = %model_dir.display(),
                        "flushed uploaded model files to S3"
                    );
                }
                if let Err(error) = backend.flush_path(&core_store_path).await {
                    tracing::warn!(
                        path = %core_store_path.display(),
                        error = %error,
                        "failed to flush model registry to S3"
                    );
                }
            });
        }
    }
}

#[cfg(feature = "s3-persistence")]
fn uploaded_model_dir(root_dir: &Path, alias: &str) -> Option<PathBuf> {
    let alias = alias.trim();
    if alias.is_empty()
        || alias.contains(['/', '\\'])
        || Path::new(alias)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return None;
    }
    Some(root_dir.join("models").join(alias))
}

fn component_response_to_http(response: ComponentResponse) -> Response {
    let mut http_response = Response::new(Body::from(Bytes::from(response.body)));
    *http_response.status_mut() = response.status;

    match fields_to_header_map(&response.headers, "header") {
        Ok(headers) => *http_response.headers_mut() = headers,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("storage component returned invalid response headers: {error}"),
            )
                .into_response();
        }
    }

    if !response.trailers.is_empty() {
        match fields_to_header_map(&response.trailers, "trailer") {
            Ok(trailers) => {
                http_response.extensions_mut().insert(trailers);
            }
            Err(error) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("storage component returned invalid response trailers: {error}"),
                )
                    .into_response();
            }
        }
    }

    http_response
}

fn header_map_to_fields(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            let value = value
                .to_str()
                .map(str::to_owned)
                .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).into_owned());
            (name.as_str().to_owned(), value)
        })
        .collect()
}

fn fields_to_header_map(
    fields: &[(String, String)],
    label: &str,
) -> std::result::Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, value) in fields {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("invalid {label} name `{name}`: {error}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|error| format!("invalid {label} value for `{name}`: {error}"))?;
        headers.append(header_name, header_value);
    }
    Ok(headers)
}

fn hash_from_asset_uri(uri: &str) -> Result<String> {
    let hash = uri
        .strip_prefix("tachyon://")
        .ok_or_else(|| anyhow!("asset URI `{uri}` must start with `tachyon://`"))?;
    validate_hash(hash)?;
    Ok(hash.to_owned())
}

fn validate_hash(hash: &str) -> Result<()> {
    let digest = hash
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("asset hash `{hash}` must start with `sha256:`"))?;
    if digest.is_empty()
        || !digest
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        anyhow::bail!("asset hash `{hash}` must be a hexadecimal sha256 digest");
    }
    Ok(())
}

impl WasiView for StorageComponentState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for StorageComponentState {
    type Data<'a> = &'a mut Self;
}

#[cfg(all(test, feature = "ai-inference"))]
mod configured_binding_registry_tests {
    use super::*;
    use crate::{IntegrityModelBinding, IntegrityRoute};

    fn temp_store() -> (crate::store::CoreStore, std::path::PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-binding-registry-{}-{nanos}",
            std::process::id()
        ));
        let store = crate::store::CoreStore::open(&dir.join("core.redb")).expect("store opens");
        (store, dir)
    }

    fn config_with(bindings: Vec<IntegrityModelBinding>) -> crate::IntegrityConfig {
        crate::IntegrityConfig {
            routes: vec![IntegrityRoute {
                models: bindings,
                ..IntegrityRoute::default()
            }],
            ..crate::IntegrityConfig::default()
        }
    }

    fn binding(alias: &str, path: &str, dynamic: bool) -> IntegrityModelBinding {
        IntegrityModelBinding {
            alias: alias.to_owned(),
            path: path.to_owned(),
            device: crate::ModelDevice::Cpu,
            qos: crate::RouteQos::Standard,
            dynamic,
            hardware_strategy: Default::default(),
        }
    }

    #[test]
    fn engine_label_identifies_an_upstream_binding() {
        assert_eq!(
            binding_engine_label("openai:http://127.0.0.1:8080/v1"),
            "openai"
        );
        assert_eq!(binding_engine_label("mock:demo"), "mock");
        assert_eq!(binding_engine_label("mock"), "mock");
        // A directory that does not exist cannot be probed for a `.gguf`, so it
        // falls back to the safetensors label rather than guessing.
        assert_eq!(
            binding_engine_label("/models/does-not-exist"),
            "safetensors"
        );
    }

    #[test]
    fn configured_bindings_become_visible_in_the_model_registry() {
        let (store, dir) = temp_store();
        let config = config_with(vec![
            binding("remote-coder", "openai:http://127.0.0.1:8080/v1", false),
            // Dynamic bindings are registered by the upload that materialises
            // them, so publishing a config-derived row here would be wrong.
            binding("uploaded-later", "", true),
        ]);

        publish_configured_model_bindings(&store, &config);

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "remote-coder")
            .expect("registry read should succeed")
            .expect("the upstream binding should be registered");
        let entry: serde_json::Value =
            serde_json::from_slice(&raw).expect("registry row should be JSON");
        assert_eq!(entry["alias"], "remote-coder");
        assert_eq!(entry["engine"], "openai");
        assert_eq!(entry["status"], "available");
        // camelCase, or `guest-openai`'s reader silently drops the row.
        assert_eq!(entry["vramRequiredMb"], 0);

        assert!(store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "uploaded-later")
            .expect("registry read should succeed")
            .is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_uploaded_entry_is_never_clobbered_by_a_configured_binding() {
        let (store, dir) = temp_store();
        // An upload-published row carries a real path and VRAM figure; the
        // config-derived one knows neither, so it must not overwrite it.
        let uploaded = serde_json::json!({
            "alias": "shared", "engine": "gguf",
            "vramRequiredMb": 4096, "status": "available",
            "modelPath": "/data/tachyon_data/models/shared",
        });
        store
            .kv_partition_set(
                AI_MODELS_REGISTRY_TABLE,
                "shared",
                &serde_json::to_vec(&uploaded).expect("serialize"),
            )
            .expect("seed write should succeed");

        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "shared",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "shared")
            .expect("registry read should succeed")
            .expect("row should still exist");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("row should be JSON");
        assert_eq!(entry["engine"], "gguf");
        assert_eq!(entry["vramRequiredMb"], 4096);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_configured_row_is_refreshed_when_its_binding_kind_changes() {
        let (store, dir) = temp_store();
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "coder",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );
        // Hot reload swaps the same alias to a local checkpoint: leaving the
        // old row would keep advertising `openai/coder`.
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding("coder", "/models/coder", false)]),
        );

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "coder")
            .expect("read")
            .expect("row");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(entry["engine"], "safetensors");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_configured_row_is_dropped_when_its_binding_leaves_the_manifest() {
        let (store, dir) = temp_store();
        publish_configured_model_bindings(
            &store,
            &config_with(vec![
                binding("keep", "openai:http://127.0.0.1:8080/v1", false),
                binding("drop", "openai:http://127.0.0.1:8081/v1", false),
            ]),
        );
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "keep",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );

        assert!(store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "keep")
            .expect("read")
            .is_some());
        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "drop")
                .expect("read")
                .is_none(),
            "a removed binding must stop being advertised"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_uploaded_row_is_never_refreshed_or_swept() {
        let (store, dir) = temp_store();
        let uploaded = serde_json::json!({
            "alias": "shared", "engine": "gguf", "vramRequiredMb": 4096,
            "status": "available", "modelPath": "/data/models/shared",
        });
        store
            .kv_partition_set(
                AI_MODELS_REGISTRY_TABLE,
                "shared",
                &serde_json::to_vec(&uploaded).expect("serialize"),
            )
            .expect("seed");

        // Present in the manifest, then absent: neither pass may touch it.
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "shared",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );
        publish_configured_model_bindings(&store, &config_with(Vec::new()));

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "shared")
            .expect("read")
            .expect("an uploaded row must survive");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(entry["engine"], "gguf");
        assert_eq!(entry["vramRequiredMb"], 4096);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn engine_label_classifies_the_trimmed_path() {
        // `UpstreamEndpoint::parse` trims before matching the scheme, so a
        // padded path loads as an upstream; the label must agree.
        assert_eq!(
            binding_engine_label("  openai:http://127.0.0.1:8080/v1 "),
            "openai"
        );
    }

    #[test]
    fn publishing_is_idempotent_across_restarts() {
        let (store, dir) = temp_store();
        let config = config_with(vec![binding(
            "remote-coder",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);

        publish_configured_model_bindings(&store, &config);
        let first = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "remote-coder")
            .expect("read")
            .expect("row");
        publish_configured_model_bindings(&store, &config);
        let second = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "remote-coder")
            .expect("read")
            .expect("row");

        assert_eq!(first, second);
        let _ = fs::remove_dir_all(dir);
    }
}

#[cfg(all(test, feature = "s3-persistence"))]
mod tests {
    use super::*;

    #[test]
    fn uploaded_model_dir_accepts_single_component_alias() {
        let root = Path::new("/tachyon/tachyon_data");
        assert_eq!(
            uploaded_model_dir(root, "llama-3"),
            Some(PathBuf::from("/tachyon/tachyon_data/models/llama-3"))
        );
    }

    #[test]
    fn uploaded_model_dir_rejects_path_like_aliases() {
        let root = Path::new("/tachyon/tachyon_data");
        assert!(uploaded_model_dir(root, "").is_none());
        assert!(uploaded_model_dir(root, "../llama").is_none());
        assert!(uploaded_model_dir(root, "models/llama").is_none());
        assert!(uploaded_model_dir(root, r"models\llama").is_none());
    }
}

#[cfg(test)]
mod registry_casing_tests {
    use super::*;
    use serde::Deserialize;

    /// Mirror of `guest-openai`'s `ModelInfo` reader, which owns the
    /// `ai-models-registry` table and reads rows with `#[serde(rename_all =
    /// "camelCase")]` and a *required* `vram_required_mb`. If the host writer
    /// drifts back to snake_case, deserialization fails here exactly as it does
    /// in `guest-openai::list_models` (which silently `filter_map(...ok())`s the
    /// miss), making uploaded models invisible in `GET /ai/v1/models`.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(dead_code)]
    struct GuestOpenAiModelInfoReader {
        alias: String,
        engine: String,
        vram_required_mb: u64,
        status: String,
    }

    #[test]
    fn registry_entry_is_readable_by_guest_openai_camelcase_reader() {
        let info = RegistryModelInfo {
            alias: "tinyllama",
            engine: "gguf",
            vram_required_mb: 0,
            status: "available",
            model_path: "/data/tachyon_data/models/tinyllama",
            source: None,
        };
        let bytes = serde_json::to_vec(&info).expect("serialize registry entry");

        // Reproduces `guest-openai::list_models`, which does
        // `serde_json::from_slice::<ModelInfo>(&v)` and drops any miss.
        let parsed: GuestOpenAiModelInfoReader = serde_json::from_slice(&bytes)
            .expect("guest-openai must be able to read host-written registry rows");
        assert_eq!(parsed.alias, "tinyllama");
        assert_eq!(parsed.engine, "gguf");
        assert_eq!(parsed.vram_required_mb, 0);
        assert_eq!(parsed.status, "available");

        // Lock the on-the-wire key casing too, so the contract is explicit.
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).expect("registry entry must be valid JSON");
        assert!(
            value.get("vramRequiredMb").is_some(),
            "registry entry must serialize camelCase `vramRequiredMb`"
        );
        assert!(
            value.get("modelPath").is_some(),
            "registry entry must serialize camelCase `modelPath`"
        );
        assert!(
            value.get("vram_required_mb").is_none(),
            "registry entry must not emit snake_case keys"
        );
    }
}
