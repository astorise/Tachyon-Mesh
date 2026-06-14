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

pub(crate) async fn upload_asset_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, REGISTRY_MODULE_NAME, asset_registry_dir).await
}

pub(crate) async fn init_upload_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, MODEL_BROKER_MODULE_NAME, model_broker_dir).await
}

pub(crate) async fn upload_chunk_handler(
    State(state): State<crate::AppState>,
    request: Request,
) -> Response {
    proxy_request_to_component(state, request, MODEL_BROKER_MODULE_NAME, model_broker_dir).await
}

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
    let engine = state.runtime.load().engine.clone();
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
            component_request,
        )
    })
    .await
    {
        Ok(Ok(response)) => component_response_to_http(response),
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
    let component =
        crate::load_component_with_core_store(engine, &module_path, &core_store, "default")
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
    let bindings = bindings::SystemFaasGuest::instantiate(&mut store, &component, &linker)
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
// models never surface in `GET /v1/models`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryModelInfo<'a> {
    alias: &'a str,
    engine: &'a str,
    vram_required_mb: u64,
    status: &'a str,
    model_path: &'a str,
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
    /// miss), making uploaded models invisible in `GET /v1/models`.
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
