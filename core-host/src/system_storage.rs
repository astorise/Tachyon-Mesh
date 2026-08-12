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

pub(crate) const AI_MODELS_REGISTRY_TABLE: &str = "ai-models-registry";

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
    /// Tool-call dialect this checkpoint emits, resolved from its sidecar or
    /// its chat template. `guest-openai` reads it to pick a parser instead of
    /// pattern-matching the alias, which is what made tool calling depend on
    /// how a model happened to be named. Absent when the model does not
    /// declare one, or is an upstream binding (which applies its own template
    /// and returns already-structured calls).
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_parser: Option<&'a str>,
    /// Set on a *reservation* row: the alias is still the manifest's, but no
    /// runtime is serving it right now. Written for the length of a hot-reload
    /// swap, and overwritten by the real row when publication follows.
    ///
    /// `guest-openai` filters these out, so the alias 404s while it is set —
    /// the transient absence the withdrawal wants — while `row_is_config_owned`
    /// still answers `true`, so an upload cannot claim the alias in the gap.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    withdrawn: bool,
}

/// The tool-call dialect to advertise for a configured binding.
///
/// Only local checkpoints are probed: an `openai:` upstream returns structured
/// `tool_calls` of its own and never needs its text parsed, and `mock` has no
/// checkpoint to read.
///
/// Present in every build, not just `ai-inference` ones: the upload path that
/// calls it is not feature-gated. Without the detector there is nothing to
/// classify a checkpoint with, so the field stays absent and `guest-openai`
/// falls back to its own resolution — the same behaviour as a model that
/// declares nothing.
pub(crate) fn binding_tool_call_parser(path: &str) -> Option<&'static str> {
    let path = path.trim();
    if path == "mock" || path.starts_with("mock:") {
        return None;
    }
    #[cfg(feature = "ai-inference")]
    {
        if path.starts_with(crate::ai_inference::UPSTREAM_SCHEME) {
            return None;
        }
        crate::ai_inference::detect_tool_call_parser(std::path::Path::new(path))
    }
    #[cfg(not(feature = "ai-inference"))]
    {
        None
    }
}

/// Marks a registry row as derived from the manifest rather than an upload.
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
        // The sidecar wins, because `resolve_model_format` treats it as
        // authoritative when loading. Probing extensions first made the public
        // `{engine}/{alias}` id disagree with the backend actually selected —
        // a directory declaring `safetensors` while still holding a stale
        // `.gguf` advertised itself as `gguf/<alias>` and then loaded
        // safetensors.
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
        // ONNX outranks the sidecar, because the loader never asks the sidecar
        // about it. `CandleEmbeddingRuntime::try_load` runs first and resolves
        // its file by `model_file`, then `model.onnx`, then any `.onnx` in the
        // directory — the declared format is consulted nowhere in that path. A
        // directory declaring `safetensors` beside a usable ONNX therefore
        // loaded the embedding backend while advertising `safetensors/<alias>`,
        // and the label is half the public `{engine}/{alias}` id, not just
        // metadata.
        //
        // "Usable" is that runtime's own bar: without `tokenizer.json` it
        // declines and the next probe takes over, so the declaration is honest
        // again and wins below.
        if has_onnx && std::path::Path::new(path).join("tokenizer.json").is_file() {
            return "onnx";
        }
        if let Some(declared) = declared_model_format(path) {
            return declared;
        }
        // ONNX first, because that is the order `CandleBackendModel::load`
        // probes in: `CandleEmbeddingRuntime::try_load` runs before the GGUF
        // runtime and accepts a bare `.onnx` file with no sidecar. Preferring
        // GGUF here labelled a directory `gguf/<alias>` while requests for it
        // executed the ONNX embedding backend — and the label is half the
        // public `{engine}/{alias}` id, not just metadata.
        match (has_onnx, has_gguf) {
            (true, _) => "onnx",
            (false, true) => "gguf",
            (false, false) => "safetensors",
        }
    }
}

/// The format a model directory declares in its `.tachyon-model.json` sidecar,
/// when it declares one this publisher can name.
///
/// Gated with its only caller: `binding_engine_label` classifies manifest
/// bindings, which only exist on an `ai-inference` build, and CI compiles the
/// default build with `-D dead_code`.
#[cfg(feature = "ai-inference")]
fn declared_model_format(path: &str) -> Option<&'static str> {
    #[derive(serde::Deserialize)]
    struct Sidecar {
        #[serde(default)]
        format: String,
    }

    let raw = fs::read(std::path::Path::new(path).join(".tachyon-model.json")).ok()?;
    let sidecar: Sidecar = serde_json::from_slice(&raw).ok()?;
    match sidecar.format.trim().to_ascii_lowercase().as_str() {
        "gguf" => Some("gguf"),
        "safetensors" => Some("safetensors"),
        // Omitting this let a directory declaring ONNX but still holding a
        // stale `.gguf` fall through to extension probing, which gives GGUF
        // precedence — while the loader tries the embedding runtime first and
        // honours the sidecar. The row then advertised `gguf/<alias>` for a
        // backend that is ONNX.
        "onnx" => Some("onnx"),
        // Anything else — absent, empty, or a value the loader would itself
        // reject — falls through to probing, which is what happened before the
        // sidecar existed.
        _ => None,
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
/// Returns how many registry transactions failed, so a caller that placed
/// withdrawal reservations before the swap can tell whether they were lifted.
/// Zero means the registry now describes the runtime that is answering.
pub(crate) fn publish_configured_model_bindings(
    core_store: &crate::store::CoreStore,
    config: &crate::IntegrityConfig,
) -> usize {
    let mut failures = 0usize;
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
                tool_call_parser: binding_tool_call_parser(&binding.path),
                withdrawn: false,
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
                    // Counted, not just logged: the caller's whole reason for
                    // reading this number is to decide whether the withdrawal
                    // reservations it placed before the swap were lifted. A
                    // transaction that failed here left the alias reserved and
                    // unpublished, which is exactly the state the retry exists
                    // for — reporting zero would end the reload with the alias
                    // withdrawn and no row describing it.
                    failures += 1;
                    tracing::warn!(
                        alias = %binding.alias,
                        "failed to publish configured model binding to the registry: {error:#}"
                    );
                    continue;
                }
            }

            // A row already exists, and this binding is non-dynamic — which
            // means the runtime has *already* eagerly loaded it under this
            // alias, and `ensure_model_loaded` will short-circuit for that key
            // forever after. Whatever the row says, a request for this alias
            // executes the configured backend.
            //
            // So the row is overwritten even when an upload owns it. Keeping
            // the upload row would advertise (say) `gguf/shared` with the
            // uploaded checkpoint's path while the alias actually runs the
            // configured binding — and `guest-openai` strips the engine prefix
            // back to the bare alias, so a client selecting the advertised
            // upload would have its prompt answered by a different model, in
            // the `openai:` case by a third-party server. A listing that lies
            // about where a prompt goes is worse than one that loses an
            // upload's VRAM figure.
            //
            // The write is still a single transaction: a read followed by a
            // write would let an upload land in between and be reported as the
            // winner when it is not.
            // Overwriting is right, but *losing* the upload is not. The row is
            // an uploaded checkpoint's only registry metadata; delete it and
            // the files stay on disk with nothing pointing at them, so when the
            // configured binding later leaves the manifest the sweep below
            // removes the config row and the upload is gone from the listing
            // for good — recoverable only by uploading it again. The displaced
            // row therefore rides along inside the config row, and the sweep
            // puts it back instead of deleting.
            if let Err(error) = core_store.kv_partition_update(
                AI_MODELS_REGISTRY_TABLE,
                &binding.alias,
                |current| {
                    let shadowed = current.filter(|_| !row_is_config_owned(current));
                    if let Some(shadowed) = shadowed {
                        tracing::warn!(
                            alias = %binding.alias,
                            "an uploaded model shares an alias with a configured binding; the configured binding is what executes, so the registry row now describes it and the upload's row is held aside"
                        );
                        if let Some(carried) = carry_shadowed_upload(&value, shadowed) {
                            return crate::store::KvPartitionUpdate::Set(carried);
                        }
                        return crate::store::KvPartitionUpdate::Set(value);
                    }
                    // The row is already this publisher's, which is the state
                    // every reload after the first one finds. If a previous
                    // reload parked an upload inside it, the fresh row does not
                    // carry it — so writing `value` plain would drop the only
                    // saved copy of the displaced upload's metadata, and the
                    // sweep would later delete the config row with nothing to
                    // restore. Displacing an upload once must not become
                    // deleting it on the next reload.
                    if let Some(parked) = current.and_then(shadowed_upload_row) {
                        if let Some(carried) = carry_shadowed_upload(&value, &parked) {
                            return crate::store::KvPartitionUpdate::Set(carried);
                        }
                    }
                    crate::store::KvPartitionUpdate::Set(value)
                },
            ) {
                failures += 1;
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
    let owned = match config_owned_aliases(core_store) {
        Ok(owned) => owned,
        Err(error) => {
            // Counted, so the reload retries instead of accepting a sweep that
            // never ran. Nothing has been deleted on this path yet, so stopping
            // here leaves the registry exactly as the publication above left it.
            tracing::warn!("failed to scan the model registry for stale bindings: {error}");
            return failures + 1;
        }
    };
    for alias in owned {
        if configured.contains(alias.as_str()) {
            continue;
        }
        // Same race: an upload may have replaced this row since the scan, so
        // ownership is re-checked inside the deleting transaction.
        if let Err(error) =
            core_store.kv_partition_update(AI_MODELS_REGISTRY_TABLE, &alias, |current| {
                if !row_is_config_owned(current) {
                    return crate::store::KvPartitionUpdate::Keep;
                }
                // An upload this binding displaced comes back rather than
                // going down with it. Its files never moved; only the row that
                // made them findable did.
                match current.and_then(shadowed_upload_row) {
                    Some(restored) => crate::store::KvPartitionUpdate::Set(restored),
                    None => crate::store::KvPartitionUpdate::Delete,
                }
            })
        {
            failures += 1;
            tracing::warn!(
                %alias,
                "failed to drop a stale configured model binding from the registry: {error:#}"
            );
        } else {
            tracing::info!(%alias, "dropped a configured model binding that left the manifest");
        }
    }
    failures
}

/// Write an uploaded model's registry row, unless a configured binding owns
/// the alias.
///
/// The single place this rule is enforced, because it has to hold for *every*
/// writer: the storage-proxy host, the general component host, and any future
/// one. A non-dynamic configured binding is loaded eagerly at boot, so
/// `ensure_model_loaded` short-circuits for its alias and a request runs that
/// backend no matter what the registry says — an `openai:` upstream included.
/// A row claiming otherwise sends a client's prompt somewhere it did not
/// choose.
///
/// Ownership is read inside the write transaction: checking first and writing
/// after would let a concurrent reconciliation land in between.
pub(crate) fn write_uploaded_model_row(
    core_store: &crate::store::CoreStore,
    alias: &str,
    value: Vec<u8>,
) -> std::result::Result<(), String> {
    let mut alias_taken = false;
    core_store
        .kv_partition_update(AI_MODELS_REGISTRY_TABLE, alias, |current| {
            if row_is_config_owned(current) {
                alias_taken = true;
                crate::store::KvPartitionUpdate::Keep
            } else {
                crate::store::KvPartitionUpdate::Set(value)
            }
        })
        .map_err(|error| format!("failed to publish model upload event: {error:#}"))?;
    if alias_taken {
        // Failing beats silently keeping the configured row: the upload cannot
        // be served under this alias whatever we write, and an error is the
        // only thing that tells the operator so.
        return Err(format!(
            "model alias `{alias}` is claimed by a configured binding in the sealed manifest; \
             uploaded models must use a free alias, or the binding must be declared `dynamic`"
        ));
    }
    Ok(())
}

/// Apply a guest's write to the model registry, refusing rows a configured
/// binding owns.
///
/// The rule itself is not new — `guest-openai` checks ownership before it
/// registers, and `write_uploaded_model_row` checks it before an upload lands.
/// What is new is *where*: a guest's `get` and `set` are two separate host
/// transactions, so a reload publishing a configured row between them left the
/// unconditional `set` to overwrite it, and the registry then advertised a
/// registration while the configured backend answered — a prompt routed to an
/// upstream the client never selected.
///
/// Enforcing it here closes that window and makes the rule hold for *any*
/// component reaching this table, not only the one that remembers to ask.
///
/// Returns the sentinel below as its error so the guest can answer 409 rather
/// than a blanket 500; `result<_, string>` is the only channel WIT gives us.
pub(crate) fn apply_guest_registry_write(
    core_store: &crate::store::CoreStore,
    table: &str,
    key: &str,
    value: Option<Vec<u8>>,
) -> std::result::Result<(), String> {
    if table != AI_MODELS_REGISTRY_TABLE {
        return match value {
            Some(value) => core_store
                .kv_partition_set(table, key, &value)
                .map_err(|error| format!("{error:#}")),
            None => core_store
                .kv_partition_delete(table, key)
                .map_err(|error| format!("{error:#}")),
        };
    }
    reject_forged_config_marker(key, value.as_ref())?;
    let mut alias_taken = false;
    core_store
        .kv_partition_update(AI_MODELS_REGISTRY_TABLE, key, |current| {
            if row_is_config_owned(current) {
                alias_taken = true;
                crate::store::KvPartitionUpdate::Keep
            } else {
                match value.clone() {
                    Some(value) => crate::store::KvPartitionUpdate::Set(value),
                    None => crate::store::KvPartitionUpdate::Delete,
                }
            }
        })
        .map_err(|error| format!("{error:#}"))?;
    if alias_taken {
        return Err(format!(
            "{GUEST_REGISTRY_ALIAS_TAKEN}: model alias `{key}` is claimed by a configured \
             binding in the sealed manifest"
        ));
    }
    Ok(())
}

/// Apply a guest's `batch-set` to the model registry, all-or-nothing.
///
/// The ownership rule and the atomicity `batch-set` promises have to hold at
/// once. Validating per key in a loop of single-key writes satisfies the first
/// and breaks the second: the entries before the refused one are already
/// committed, leaving a half-applied batch a caller has no way to reason about.
/// One transaction, with the ownership predicate read inside it, is both.
pub(crate) fn apply_guest_registry_batch(
    core_store: &crate::store::CoreStore,
    table: &str,
    entries: &[(String, Vec<u8>)],
) -> std::result::Result<(), String> {
    if table != AI_MODELS_REGISTRY_TABLE {
        return core_store
            .kv_partition_batch_set(table, entries)
            .map_err(|error| format!("{error:#}"));
    }
    for (key, value) in entries {
        reject_forged_config_marker(key, Some(value))?;
    }
    core_store
        .kv_partition_batch_update(AI_MODELS_REGISTRY_TABLE, entries, |key, current| {
            if row_is_config_owned(current) {
                return Err(format!(
                    "{GUEST_REGISTRY_ALIAS_TAKEN}: model alias `{key}` is claimed by a \
                     configured binding in the sealed manifest"
                ));
            }
            Ok(())
        })
        .map_err(|error| format!("{error:#}"))
}

/// Refuse a guest-supplied row that claims to be the manifest publisher's.
///
/// The ownership check reads the row already stored; it says nothing about
/// what is being written. A component with registry write access could
/// therefore set a *free* alias to `{"source":"config",…}` and have
/// `row_is_config_owned` treat it as manifest-owned from then on — every later
/// upload refused, every guest update and delete refused, and no manifest
/// entry anywhere to explain why. The alias is claimed until someone edits the
/// store by hand.
///
/// Refused rather than rewritten. A guest that reads a config-owned row and
/// writes it back is already stopped by the ownership check, so the only way
/// to reach here with this marker is to mint one for an alias nobody owns —
/// never legitimate, and quietly stripping it would hide the bug or the
/// attempt that produced it.
fn reject_forged_config_marker(key: &str, value: Option<&Vec<u8>>) -> Result<(), String> {
    if row_is_config_owned(value.map(Vec::as_slice)) {
        return Err(format!(
            "model alias `{key}`: a registry row may not declare \
             `source: \"{REGISTRY_SOURCE_CONFIG}\"`; that marker belongs to the manifest \
             publisher"
        ));
    }
    Ok(())
}

/// The key a displaced upload row is parked under inside a config row.
///
/// Namespaced and camel-cased like the rest of the row so it travels with it
/// through every reader; `RegistryModelInfo` does not declare it, and the
/// guest's `ModelInfo` ignores unknown fields, so nothing downstream has to
/// learn about it.
#[cfg(feature = "ai-inference")]
const SHADOWED_UPLOAD_KEY: &str = "shadowedUpload";

/// Fold `shadowed` into `value` so a later sweep can put it back.
///
/// `None` when either side is not a JSON object, in which case the caller
/// writes the config row plain — losing the upload's row is bad, but writing a
/// malformed one in its place would be worse.
#[cfg(feature = "ai-inference")]
fn carry_shadowed_upload(value: &[u8], shadowed: &[u8]) -> Option<Vec<u8>> {
    let mut row = serde_json::from_slice::<serde_json::Value>(value).ok()?;
    let shadowed = serde_json::from_slice::<serde_json::Value>(shadowed).ok()?;
    // A config row displacing another config row carries nothing: only an
    // upload's row is at risk of being the last copy.
    row.as_object_mut()?
        .insert(SHADOWED_UPLOAD_KEY.to_owned(), shadowed);
    serde_json::to_vec(&row).ok()
}

/// The upload row a config row is holding aside, ready to be written back.
#[cfg(feature = "ai-inference")]
fn shadowed_upload_row(row: &[u8]) -> Option<Vec<u8>> {
    let row = serde_json::from_slice::<serde_json::Value>(row).ok()?;
    let shadowed = row.get(SHADOWED_UPLOAD_KEY)?;
    // Never restore something that would claim the publisher's own marker: the
    // row was an upload's when it was set aside and must still read as one.
    let restored = serde_json::to_vec(shadowed).ok()?;
    (!row_is_config_owned(Some(restored.as_slice()))).then_some(restored)
}

/// Marker the guest matches on to turn an ownership refusal into a 409.
pub(crate) const GUEST_REGISTRY_ALIAS_TAKEN: &str = "model-alias-claimed-by-config";

/// Whether a registry row's raw bytes say the manifest publisher wrote it.
/// Takes the value rather than the key so the caller can decide inside a
/// transaction.
///
/// Present in every build, not just `ai-inference` ones: the upload path reads
/// it to refuse an alias a configured binding owns, and that path is not
/// feature-gated. On a build with no inference feature nothing writes
/// `source: "config"`, so it simply always answers `false`.
fn row_is_config_owned(row: Option<&[u8]>) -> bool {
    row.and_then(|raw| serde_json::from_slice::<serde_json::Value>(raw).ok())
        .and_then(|row| {
            row.get("source")
                .and_then(serde_json::Value::as_str)
                .map(|source| source == REGISTRY_SOURCE_CONFIG)
        })
        .unwrap_or(false)
}

/// Whether the registry row for `alias` still matches what publishing `path`
/// would write today.
///
/// Compares the two fields the publisher derives from the directory's
/// contents rather than from the manifest — `engine` and `tool_call_parser` —
/// so a checkpoint replaced in place is seen for what it is. A missing row is
/// "not current": there is nothing to keep listed, and withdrawing reserves
/// the alias for the publication that follows the swap.
#[cfg(feature = "ai-inference")]
fn stored_row_still_current(core_store: &crate::store::CoreStore, alias: &str, path: &str) -> bool {
    let Ok(Some(raw)) = core_store.kv_partition_get(AI_MODELS_REGISTRY_TABLE, alias) else {
        return false;
    };
    let Ok(row) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    let stored = |field: &str| {
        row.get(field)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    // `RegistryModelInfo` is `rename_all = "camelCase"`, so the stored key is
    // `toolCallParser`. Reading the Rust field name here would have found
    // nothing every time and withdrawn every parser-declaring binding on every
    // reload — a silent availability cost, in the code meant to avoid one.
    stored("engine").as_deref() == Some(binding_engine_label(path))
        && stored("toolCallParser").as_deref() == binding_tool_call_parser(path)
}

/// Withdraw configured rows the incoming config will not serve identically.
///
/// Called *before* a hot-reload swap, while the previous runtime is still
/// answering. Publication necessarily happens after the swap — the new aliases
/// are not loadable until then — so without this the window between the two
/// advertises the old runtime's rows against the new runtime's behaviour. For
/// an alias whose engine changed that is not a cosmetic lag: the listing offers
/// `gguf/<alias>` while requests reach an `openai:` upstream.
///
/// Only rows that would *change* are withdrawn. An alias whose binding is
/// identical on both sides keeps its row throughout, so an unrelated reload
/// costs no availability.
#[cfg(feature = "ai-inference")]
pub(crate) fn withdraw_changed_model_bindings(
    core_store: &crate::store::CoreStore,
    previous: &crate::IntegrityConfig,
    incoming: &crate::IntegrityConfig,
) {
    let publishable = |config: &crate::IntegrityConfig| {
        config
            .routes
            .iter()
            .flat_map(|route| route.models.iter())
            .filter(|binding| !binding.dynamic && !binding.path.trim().is_empty())
            .map(|binding| (binding.alias.clone(), binding.path.clone()))
            .collect::<std::collections::HashMap<_, _>>()
    };
    let incoming = publishable(incoming);

    for (alias, path) in publishable(previous) {
        // Same alias, same path, *and* the stored row still says what a fresh
        // publication would say.
        //
        // The path alone used to decide this, and the row it stands for is not
        // a function of the path: the publisher derives `engine` and
        // `tool_call_parser` from the directory's contents, which a checkpoint
        // swapped in place behind an unchanged path changes underneath it. The
        // comparison also has to be against the *stored* row rather than
        // against the incoming config, because both configs are read here at
        // the same instant and would agree about a directory neither of them
        // describes. The stored row is the only witness of what the last
        // publication saw.
        //
        // Getting this wrong kept the old row listed while the new backend was
        // already serving, and if the best-effort publication after the swap
        // then failed, it stayed listed indefinitely.
        if incoming.get(&alias).is_some_and(|next| *next == path)
            && stored_row_still_current(core_store, &alias, &path)
        {
            continue;
        }
        // An alias the incoming config still owns is *reserved*, not released.
        // Deleting it left the row unclaimed for the whole swap — worker
        // replacement and runtime store, not a short window — and an upload
        // committing in there succeeded, was reported installed, and was then
        // overwritten by the incoming binding's publication. If the binding
        // uses the broker directory, that upload's files also landed under a
        // runtime that had not asked for them.
        //
        // The reservation is config-owned, so the upload path refuses it, and
        // withdrawn, so the alias is not advertised until publication replaces
        // it.
        //
        // Placed for every alias being withdrawn, including the ones the
        // incoming config drops. Deleting those outright opened the same window
        // one step later: the alias reads as free while the *previous* runtime
        // is still the one answering, so an upload could commit, be advertised,
        // and take requests the configured backend was still serving. Holding
        // the reservation across the swap is not a leak, because the
        // publication that follows sweeps every config-owned row the incoming
        // manifest does not claim — which is exactly this one.
        let reservation = Some(serde_json::to_vec(&RegistryModelInfo {
            alias: &alias,
            engine: "",
            vram_required_mb: 0,
            status: "reloading",
            model_path: "",
            source: Some(REGISTRY_SOURCE_CONFIG),
            tool_call_parser: None,
            withdrawn: true,
        }));
        let reservation = match reservation {
            Some(Ok(value)) => Some(value),
            // Encoding cannot realistically fail, and if it did, releasing the
            // alias is still better than leaving the previous row advertising a
            // runtime that is about to stop answering.
            Some(Err(_)) | None => None,
        };
        // Ownership is re-read inside the transaction: an upload that landed
        // since the scan owns its row, and a reload must not touch it.
        //
        // An *absent* row is not an upload's, and it is the case
        // `stored_row_still_current` deliberately routes here — a binding whose
        // publication never landed still has a runtime answering for it, so the
        // alias needs sealing exactly like a stale one. Treating absent as
        // "not ours" left it free for the whole swap, which is the window the
        // reservation exists to close: an upload commits into it, is reported
        // installed, and is then displaced by the publication that follows.
        if let Err(error) =
            core_store.kv_partition_update(AI_MODELS_REGISTRY_TABLE, &alias, |current| {
                if current.is_some() && !row_is_config_owned(current) {
                    return crate::store::KvPartitionUpdate::Keep;
                }
                match reservation.clone() {
                    // The reservation replaces the whole row, so an upload this
                    // binding had displaced would go down with it: the
                    // publication after the swap either overwrites the
                    // reservation or sweeps it, and neither can put back what
                    // the reservation erased. The files stay on disk and vanish
                    // from the listing for good. Carried across, exactly as the
                    // refresh path carries it.
                    Some(value) => {
                        let carried = current
                            .and_then(shadowed_upload_row)
                            .and_then(|parked| carry_shadowed_upload(&value, &parked));
                        crate::store::KvPartitionUpdate::Set(carried.unwrap_or(value))
                    }
                    // Nothing to withdraw and nothing writable to reserve with.
                    None if current.is_none() => crate::store::KvPartitionUpdate::Keep,
                    None => crate::store::KvPartitionUpdate::Delete,
                }
            })
        {
            tracing::warn!(
                %alias,
                "failed to withdraw a changed model binding before the reload swap: {error:#}"
            );
        }
    }
}

/// Every alias in the registry whose row this publisher owns.
///
/// `Err` when a page could not be read. A failed read used to become an empty
/// page, which is indistinguishable from the end of the table: the sweep then
/// finished early, reported nothing wrong, and the reload's retry loop returned
/// satisfied — leaving aliases the manifest had dropped still advertised in
/// `GET /ai/v1/models` with no runtime behind them. Under-sweeping is not itself
/// destructive, so the answer is to say the scan was incomplete and be retried,
/// rather than to act on a partial view.
#[cfg(feature = "ai-inference")]
fn config_owned_aliases(core_store: &crate::store::CoreStore) -> Result<Vec<String>, String> {
    /// Rows per read. The page size is a transaction-size bound, not a limit
    /// on how much of the table this sweep covers.
    const PAGE: u32 = 10_000;

    let mut owned = Vec::new();
    let mut offset = 0u32;
    loop {
        // A single page used to be the whole sweep, so once the table grew
        // past it — uploaded rows count too — any configured alias beyond that
        // page stayed advertised forever after leaving the manifest.
        let page = core_store
            .kv_partition_get_range(AI_MODELS_REGISTRY_TABLE, "", "\u{10ffff}", PAGE, offset)
            .map_err(|error| {
                format!("failed to read the model registry at offset {offset}: {error:#}")
            })?;
        let read = page.len() as u32;
        owned.extend(page.into_iter().filter_map(|(alias, raw)| {
            let row = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
            (row.get("source").and_then(serde_json::Value::as_str) == Some(REGISTRY_SOURCE_CONFIG))
                .then_some(alias)
        }));
        if read < PAGE {
            return Ok(owned);
        }
        offset = offset.saturating_add(read);
    }
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
            tool_call_parser: binding_tool_call_parser(&event.model_path),
            withdrawn: false,
        };
        let value = serde_json::to_vec(&info)
            .map_err(|error| format!("failed to encode model registry entry: {error}"))?;
        tracing::info!(
            alias = %event.alias,
            engine = %event.engine,
            model_path = %event.model_path,
            "publishing uploaded model to registry `{AI_MODELS_REGISTRY_TABLE}`"
        );
        // Refuse to take an alias a configured binding already owns.
        //
        // `publish_configured_model_bindings` resolves that collision at boot
        // and on reload, but an upload committing *afterwards* would overwrite
        // the row while the runtime kept executing the eagerly loaded
        // configured backend — so the listing would advertise this uploaded
        // checkpoint and requests for the alias would still reach the
        // configured model, an `openai:` upstream included.
        //
        // Failing is better than silently keeping the configured row: the
        // upload cannot be served under this alias whatever we write, and an
        // error is the only thing that tells the operator so. Ownership is read
        // inside the write transaction, so a concurrent reconciliation cannot
        // slip between a check and a set.
        write_uploaded_model_row(&self.core_store, &event.alias, value)?;
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
    fn a_configured_binding_owns_the_row_for_an_alias_it_already_executes() {
        let (store, dir) = temp_store();
        // The collision that matters: an upload and a *non-dynamic* manifest
        // binding claim the same alias. The runtime loaded the configured
        // binding eagerly at boot, so `ensure_model_loaded("shared")`
        // short-circuits and every request for `shared` reaches the upstream —
        // whatever the registry says.
        //
        // Leaving the upload row would advertise `gguf/shared` with an on-disk
        // path while `guest-openai` strips the prefix back to `shared`, so a
        // client picking the advertised local checkpoint would have its prompt
        // sent to a third-party server. Losing the upload's VRAM figure is the
        // cheaper error by far.
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
        assert_eq!(
            entry["engine"], "openai",
            "the listing must describe what a request for this alias actually runs"
        );
        assert_eq!(entry["modelPath"], "openai:http://127.0.0.1:8080/v1");
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
    fn an_uploaded_row_no_binding_claims_is_never_touched() {
        let (store, dir) = temp_store();
        let uploaded = serde_json::json!({
            "alias": "uploaded-only", "engine": "gguf", "vramRequiredMb": 4096,
            "status": "available", "modelPath": "/data/models/uploaded-only",
        });
        store
            .kv_partition_set(
                AI_MODELS_REGISTRY_TABLE,
                "uploaded-only",
                &serde_json::to_vec(&uploaded).expect("serialize"),
            )
            .expect("seed");

        // Neither the refresh pass nor the sweep may touch a row this publisher
        // does not own: the model is on disk, reachable, and nothing in the
        // manifest contradicts it. The sweep in particular only walks
        // config-owned aliases, so an upload must never be collateral.
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "unrelated",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );
        publish_configured_model_bindings(&store, &config_with(Vec::new()));

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "uploaded-only")
            .expect("read")
            .expect("an uploaded row must survive");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(entry["engine"], "gguf");
        assert_eq!(entry["vramRequiredMb"], 4096);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_onnx_sidecar_is_authoritative_over_a_stale_gguf_file() {
        let (_store, dir) = temp_store();
        let model_dir = dir.join("onnx-embed");
        fs::create_dir_all(&model_dir).expect("model dir");
        fs::write(
            model_dir.join(".tachyon-model.json"),
            serde_json::json!({ "format": "onnx" }).to_string(),
        )
        .expect("sidecar");
        // A leftover checkpoint from a previous upload. Extension probing gives
        // GGUF precedence, but the loader tries the embedding runtime first and
        // honours the sidecar — so the row would have advertised `gguf/<alias>`
        // for a backend that is ONNX.
        fs::write(model_dir.join("stale.gguf"), b"not read").expect("stale gguf");
        fs::write(model_dir.join("model.onnx"), b"not read").expect("onnx");

        assert_eq!(
            binding_engine_label(model_dir.to_str().expect("utf-8 path")),
            "onnx"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sidecar_less_probing_follows_the_loaders_onnx_first_order() {
        let (_store, dir) = temp_store();
        let model_dir = dir.join("onnx-no-sidecar");
        fs::create_dir_all(&model_dir).expect("model dir");
        // No sidecar at all, and a leftover checkpoint beside the ONNX one.
        // `CandleBackendModel::load` probes the embedding runtime first and it
        // accepts a bare `.onnx`, so labelling this `gguf/<alias>` advertised a
        // backend that would never run.
        fs::write(model_dir.join("stale.gguf"), b"not read").expect("stale gguf");
        fs::write(model_dir.join("model.onnx"), b"not read").expect("onnx");

        assert_eq!(
            binding_engine_label(model_dir.to_str().expect("utf-8 path")),
            "onnx"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn every_uploaded_row_write_respects_configured_ownership() {
        let (store, dir) = temp_store();
        // The rule has to hold for callers other than the storage proxy — the
        // general component host reaches the registry through the same shared
        // writer, so this exercises it directly.
        publish_configured_model_bindings(
            &store,
            &config_with(vec![binding(
                "claimed",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
        );

        let error = write_uploaded_model_row(&store, "claimed", b"{}".to_vec())
            .expect_err("a configured alias must not be taken by an upload");
        assert!(
            error.contains("claimed") && error.contains("dynamic"),
            "the rejection should name the alias and the way out, got: {error}"
        );

        // The configured row is intact, and a free alias still writes.
        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "claimed")
            .expect("read")
            .expect("row");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(entry["engine"], "openai");
        write_uploaded_model_row(&store, "free", b"{}".to_vec()).expect("a free alias is writable");
        let _ = fs::remove_dir_all(dir);
    }

    /// A checkpoint replaced behind an unchanged path is a change.
    ///
    /// The withdrawal used to compare alias and path, on the assumption that
    /// the row a path publishes is a function of the path. It is not: the
    /// publisher reads `engine` and `tool_call_parser` out of the directory,
    /// so swapping the checkpoint in place produces a different row from the
    /// same string. The old row stayed listed while the new backend served,
    /// and a failed best-effort publication after the swap left it listed for
    /// good.
    ///
    /// Comparing the two *configs* would not have caught it either — both are
    /// read at the same instant and neither describes the directory. The
    /// stored row is the only witness of what the last publication saw, which
    /// is why the reload here passes the same config on both sides.
    #[test]
    fn a_reload_withdraws_a_binding_whose_checkpoint_changed_under_its_path() {
        let (store, dir) = temp_store();
        let model_dir = dir.join("qwen-coder");
        std::fs::create_dir_all(&model_dir).expect("model dir");
        let sidecar = model_dir.join(".tachyon-model.json");
        std::fs::write(&sidecar, br#"{"tool_call_parser":"qwen"}"#).expect("write sidecar");

        let path = model_dir.to_string_lossy().into_owned();
        assert_eq!(
            binding_tool_call_parser(&path),
            Some("qwen"),
            "the sidecar is what the publisher reads the dialect from"
        );
        let config = config_with(vec![binding("qwen-coder", &path, false)]);
        publish_configured_model_bindings(&store, &config);

        let published = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "qwen-coder")
            .expect("read")
            .expect("the binding publishes a row");
        let published: serde_json::Value = serde_json::from_slice(&published).expect("row json");
        assert_eq!(
            published["toolCallParser"], "qwen",
            "the published row records the dialect the directory declared"
        );

        // Same alias, same path, different checkpoint.
        std::fs::write(&sidecar, br#"{"tool_call_parser":"mistral"}"#).expect("rewrite sidecar");
        withdraw_changed_model_bindings(&store, &config, &config);

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "qwen-coder")
            .expect("read")
            .expect("a changed binding keeps a reservation row rather than vanishing");
        let row: serde_json::Value = serde_json::from_slice(&raw).expect("row json");
        assert_eq!(
            row["withdrawn"], true,
            "a row that no longer describes what the path holds must not stay advertised"
        );
    }

    /// The converse: an untouched directory costs no availability.
    #[test]
    fn a_reload_keeps_a_binding_whose_checkpoint_is_unchanged() {
        let (store, dir) = temp_store();
        let model_dir = dir.join("qwen-coder");
        std::fs::create_dir_all(&model_dir).expect("model dir");
        std::fs::write(
            model_dir.join(".tachyon-model.json"),
            br#"{"tool_call_parser":"qwen"}"#,
        )
        .expect("write sidecar");

        let path = model_dir.to_string_lossy().into_owned();
        let config = config_with(vec![binding("qwen-coder", &path, false)]);
        publish_configured_model_bindings(&store, &config);
        withdraw_changed_model_bindings(&store, &config, &config);

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "qwen-coder")
            .expect("read")
            .expect("the row survives");
        let row: serde_json::Value = serde_json::from_slice(&raw).expect("row json");
        assert_ne!(
            row["withdrawn"], true,
            "an unchanged binding must stay listed across the reload"
        );
    }

    #[test]
    fn a_reload_withdraws_only_the_bindings_that_change() {
        let (store, dir) = temp_store();
        let previous = config_with(vec![
            binding("unchanged", "openai:http://127.0.0.1:8080/v1", false),
            binding("re-pointed", "openai:http://127.0.0.1:8080/v1", false),
            binding("removed", "openai:http://127.0.0.1:9090/v1", false),
        ]);
        publish_configured_model_bindings(&store, &previous);

        let incoming = config_with(vec![
            binding("unchanged", "openai:http://127.0.0.1:8080/v1", false),
            // Same alias, different backend: advertising the old engine while
            // the new one answers is the failure this withdrawal prevents.
            binding("re-pointed", "/models/re-pointed", false),
        ]);
        withdraw_changed_model_bindings(&store, &previous, &incoming);

        let present = |alias: &str| {
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, alias)
                .expect("read")
                .is_some()
        };
        let row = |alias: &str| {
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, alias)
                .expect("read")
                .map(|raw| serde_json::from_slice::<serde_json::Value>(&raw).expect("row json"))
        };
        assert!(
            present("unchanged"),
            "an identical binding keeps its row, so an unrelated reload costs no availability"
        );

        // A changed binding is withdrawn from *service* but not released: the
        // incoming config still owns the alias, and the swap it is waiting on
        // is long enough for an upload to land in the gap.
        let reserved = row("re-pointed").expect("a re-pointed alias keeps a reservation row");
        assert_eq!(
            reserved["withdrawn"], true,
            "a reservation must not be advertised — the alias 404s until publication"
        );
        assert_eq!(
            reserved["source"], "config",
            "and must stay config-owned, or an upload can claim it mid-swap"
        );

        // Which is the point: the same refusal an unchanged configured row
        // gets, for the whole length of the reload.
        let denied = write_uploaded_model_row(&store, "re-pointed", b"{}".to_vec())
            .expect_err("an upload must not claim an alias the incoming config still owns");
        assert!(
            denied.contains("sealed manifest"),
            "unexpected error: {denied}"
        );

        // A binding the incoming config dropped is reserved too, not released.
        // Deleting it outright opened the same window one step later: the alias
        // reads as free while the *previous* runtime is still answering for it,
        // so an upload could commit, be advertised, and take requests the
        // configured backend was still serving.
        //
        // This used to be released on the reasoning that nothing would
        // republish it, so a reservation would block the alias for good. That
        // premise was wrong: the publication after the swap sweeps every
        // config-owned row the incoming manifest does not claim, and this is
        // one of them. The cost of holding it is an alias unavailable until
        // that publication runs — the same exposure a changed alias already
        // accepts, and a smaller one than routing a prompt to a backend the
        // caller did not choose.
        assert_eq!(
            row("removed").map(|row| row["withdrawn"].clone()),
            Some(serde_json::json!(true)),
            "a removed binding is held withdrawn until the swap completes"
        );
        let denied = write_uploaded_model_row(&store, "removed", b"{}".to_vec())
            .expect_err("the reservation must hold while the previous runtime still answers");
        assert!(
            denied.contains("sealed manifest"),
            "unexpected error: {denied}"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// A scan that could not finish is reported, not treated as an empty table.
    ///
    /// `unwrap_or_default()` turned a failed page read into "no more rows",
    /// which is indistinguishable from the end of the table: the sweep stopped
    /// early, publication reported zero failures, and the reload's retry loop
    /// returned satisfied — leaving aliases the manifest had dropped still
    /// advertised with no runtime behind them.
    #[test]
    fn a_failed_registry_scan_is_a_publication_failure() {
        let (store, dir) = temp_store();
        let config = config_with(vec![binding(
            "coder",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);
        assert_eq!(publish_configured_model_bindings(&store, &config), 0);

        // A healthy table scans, and says so as `Ok` — the shape that lets the
        // publisher above tell "no stale rows" apart from "could not look".
        // Injecting a store failure needs a fake `CoreStore` this module does
        // not have, so what is proved here is the success half; the failure
        // half is carried by the signature, which no longer lets a caller read
        // an error as an empty page.
        let owned = config_owned_aliases(&store).expect("a healthy table scans");
        assert_eq!(owned, vec!["coder".to_owned()]);

        let _ = fs::remove_dir_all(dir);
    }

    /// A binding whose row never landed is sealed like any other.
    ///
    /// The alias list comes from the outgoing *config*, not from a scan, so a
    /// binding whose publication failed — or whose row an operator removed —
    /// arrives here with nothing stored. The runtime is still answering for it,
    /// which is the entire premise of the reservation; treating an absent row
    /// as "not ours" left the alias free for the whole swap, and an upload
    /// landing in that window was reported installed and then displaced by the
    /// publication that follows.
    #[test]
    fn a_configured_alias_with_no_stored_row_is_still_reserved() {
        let (store, dir) = temp_store();
        let previous = config_with(vec![binding(
            "never-published",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);
        // Deliberately no `publish_configured_model_bindings` call: the table
        // has no row for this alias.
        assert!(store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "never-published")
            .expect("read")
            .is_none());

        withdraw_changed_model_bindings(&store, &previous, &config_with(Vec::new()));

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "never-published")
            .expect("read")
            .expect("an unpublished binding is sealed on the way out");
        let row: serde_json::Value = serde_json::from_slice(&raw).expect("row json");
        assert_eq!(row["withdrawn"], true);
        assert_eq!(row["source"], "config");
        let denied = write_uploaded_model_row(&store, "never-published", b"{}".to_vec())
            .expect_err("the seal is what stops an upload claiming the alias mid-swap");
        assert!(
            denied.contains("sealed manifest"),
            "unexpected error: {denied}"
        );

        // And the publication after the swap sweeps it, so the seal is not a
        // permanent lock on an alias no config claims.
        publish_configured_model_bindings(&store, &config_with(Vec::new()));
        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "never-published")
                .expect("read")
                .is_none(),
            "a reservation no manifest claims is swept, not held forever"
        );
        let _ = fs::remove_dir_all(dir);
    }

    /// An upload a binding displaced survives the binding.
    ///
    /// The registry row is an uploaded checkpoint's only metadata. Overwriting
    /// it is right — the configured binding is what executes, so the listing
    /// has to describe that — but deleting it when the binding later leaves
    /// stranded the files: still on disk, absent from every listing, and
    /// recoverable only by uploading them again.
    #[test]
    fn an_upload_displaced_by_a_binding_comes_back_when_the_binding_leaves() {
        let (store, dir) = temp_store();

        let upload = serde_json::json!({
            "alias": "shared", "engine": "gguf", "vramRequiredMb": 4096,
            "status": "available", "modelPath": "/models/shared",
        });
        write_uploaded_model_row(
            &store,
            "shared",
            serde_json::to_vec(&upload).expect("serialize"),
        )
        .expect("a free alias accepts an upload");

        // A later manifest claims the same alias. The listing must describe the
        // backend that answers, so the config row wins.
        let config = config_with(vec![binding(
            "shared",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);
        publish_configured_model_bindings(&store, &config);
        let row = |alias: &str| {
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, alias)
                .expect("read")
                .map(|raw| serde_json::from_slice::<serde_json::Value>(&raw).expect("row json"))
        };
        let shadowing = row("shared").expect("the configured row is published");
        assert_eq!(shadowing["source"], "config");
        assert_eq!(shadowing["modelPath"], "openai:http://127.0.0.1:8080/v1");

        // The binding leaves. The upload never moved, so its row comes back
        // rather than going down with the binding that hid it.
        publish_configured_model_bindings(&store, &config_with(Vec::new()));
        let restored = row("shared").expect("the displaced upload is restored, not swept");
        assert_eq!(restored["modelPath"], "/models/shared");
        assert_eq!(restored["vramRequiredMb"], 4096);
        assert!(
            restored.get("source").is_none(),
            "the restored row is the upload's again, not the publisher's"
        );

        // The same thing, but with a reload in the middle — which is every
        // restart after the first. The refresh finds a row that is already the
        // publisher's, and writing a fresh one over it dropped the only saved
        // copy of the displaced upload: the alias then swept clean when the
        // binding left, and the files on disk became unreachable.
        write_uploaded_model_row(
            &store,
            "shared",
            serde_json::to_vec(&upload).expect("serialize"),
        )
        .expect("the restored alias is the upload's again, so it accepts a write");
        publish_configured_model_bindings(&store, &config);
        publish_configured_model_bindings(&store, &config);
        publish_configured_model_bindings(&store, &config_with(Vec::new()));
        let restored = row("shared").expect("a reload must not consume the displaced upload");
        assert_eq!(restored["modelPath"], "/models/shared");
        assert!(restored.get("source").is_none());

        // And a config row with nothing held aside is still swept, so a binding
        // that never displaced anything leaves no trace.
        let plain = config_with(vec![binding("solo", "/models/solo", false)]);
        publish_configured_model_bindings(&store, &plain);
        assert!(row("solo").is_some());
        publish_configured_model_bindings(&store, &config_with(Vec::new()));
        assert!(
            row("solo").is_none(),
            "a binding that displaced nothing is swept"
        );

        let _ = fs::remove_dir_all(dir);
    }

    /// The engine label has to name the backend that will answer.
    ///
    /// It is half the public `{engine}/{alias}` id, so a directory advertised
    /// as `safetensors/<alias>` while the ONNX embedding backend executes is a
    /// listing that lies about where a prompt goes. The sidecar is
    /// authoritative for the loaders that read it — and
    /// `CandleEmbeddingRuntime::try_load`, which runs first, never does.
    #[test]
    fn a_usable_onnx_outranks_a_sidecar_the_loader_will_not_consult() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-engine-label-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("fixture dir");
        let path = dir.to_string_lossy().to_string();

        fs::write(
            dir.join(".tachyon-model.json"),
            br#"{"format":"safetensors"}"#,
        )
        .expect("sidecar");
        assert_eq!(
            binding_engine_label(&path),
            "safetensors",
            "with nothing else to go on the declaration stands"
        );

        // An ONNX file alone is not enough: the embedding runtime needs a
        // tokenizer, and without one it declines and the declaration is honest.
        fs::write(dir.join("model.onnx"), b"not really onnx").expect("onnx");
        assert_eq!(
            binding_engine_label(&path),
            "safetensors",
            "an ONNX the embedding runtime would decline does not change what answers"
        );

        // With the tokenizer the embedding runtime claims the directory, and
        // the label has to say so.
        fs::write(dir.join("tokenizer.json"), b"{}").expect("tokenizer");
        assert_eq!(
            binding_engine_label(&path),
            "onnx",
            "the label must name the backend the loader's probe order selects"
        );

        let _ = fs::remove_dir_all(dir);
    }

    /// Publication reports what it could not write.
    ///
    /// The withdrawal before a swap leaves a reservation on every alias it
    /// touched, and this publication is what lifts them — so a caller has to be
    /// able to tell whether it did. Returning nothing meant a failed
    /// transaction left the alias hidden and refused to uploads with no way to
    /// notice, and no reconciliation anywhere to fix it before the next reload.
    #[test]
    fn publication_reports_the_transactions_it_completed() {
        let (store, dir) = temp_store();
        let config = config_with(vec![
            binding("coder", "openai:http://127.0.0.1:8080/v1", false),
            binding("embed", "/models/embed", false),
        ]);

        assert_eq!(
            publish_configured_model_bindings(&store, &config),
            0,
            "a healthy store writes every row, so nothing stays withdrawn"
        );

        // And the sweep of a dropped alias counts the same way, so a reload
        // that removes a binding can also tell whether the removal landed.
        assert_eq!(
            publish_configured_model_bindings(&store, &config_with(Vec::new())),
            0
        );
        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "coder")
                .expect("read")
                .is_none(),
            "a binding that left the manifest is swept, which is what frees its reservation"
        );

        let _ = fs::remove_dir_all(dir);
    }

    /// A guest cannot mint the manifest publisher's marker.
    ///
    /// The ownership check reads the row already stored and says nothing about
    /// what is being written, so a component could set a *free* alias to
    /// `source: "config"` and have it treated as manifest-owned from then on:
    /// uploads refused, guest updates and deletes refused, and no manifest
    /// entry anywhere to explain why. The alias would stay claimed until
    /// someone edited the store by hand.
    #[test]
    fn a_guest_cannot_forge_the_config_ownership_marker() {
        let (store, dir) = temp_store();
        let forged = serde_json::json!({
            "alias": "mine", "engine": "gguf", "vramRequiredMb": 0,
            "status": "available", "modelPath": "/models/mine",
            "source": "config",
        });

        let denied = apply_guest_registry_write(
            &store,
            AI_MODELS_REGISTRY_TABLE,
            "mine",
            Some(serde_json::to_vec(&forged).expect("serialize")),
        )
        .expect_err("a guest row may not declare itself config-owned");
        assert!(
            denied.contains("belongs to the manifest publisher"),
            "{denied}"
        );

        // Nothing landed, so the alias is still free for its rightful writer.
        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "mine")
                .expect("read")
                .is_none(),
            "a refused write must not leave a partial row behind"
        );

        // The batch path takes the same answer, and takes it for the whole
        // batch: one forged entry must not commit its neighbours.
        let honest = serde_json::json!({
            "alias": "other", "engine": "gguf", "vramRequiredMb": 0,
            "status": "available", "modelPath": "/models/other",
        });
        let denied = apply_guest_registry_batch(
            &store,
            AI_MODELS_REGISTRY_TABLE,
            &[
                (
                    "other".to_owned(),
                    serde_json::to_vec(&honest).expect("serialize"),
                ),
                (
                    "mine".to_owned(),
                    serde_json::to_vec(&forged).expect("serialize"),
                ),
            ],
        )
        .expect_err("a batch carrying a forged marker must be refused whole");
        assert!(
            denied.contains("belongs to the manifest publisher"),
            "{denied}"
        );
        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "other")
                .expect("read")
                .is_none(),
            "the honest entry beside a forged one must not commit on its own"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_guest_registration_cannot_overwrite_a_configured_row() {
        let (store, dir) = temp_store();
        let configured = config_with(vec![binding(
            "coder",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);
        publish_configured_model_bindings(&store, &configured);

        // The guest checks ownership before it writes, but the check and the
        // write are two separate host transactions — a reload publishing the
        // alias in between would slip past it. The refusal that counts is this
        // one, inside the write.
        let registration = serde_json::json!({
            "alias": "coder", "engine": "safetensors", "vramRequiredMb": 0,
            "status": "available", "modelPath": "/models/mine",
        });
        let denied = apply_guest_registry_write(
            &store,
            AI_MODELS_REGISTRY_TABLE,
            "coder",
            Some(serde_json::to_vec(&registration).expect("serialize")),
        )
        .expect_err("a configured alias must not be overwritten by a registration");
        assert!(
            denied.contains(GUEST_REGISTRY_ALIAS_TAKEN),
            "the guest turns this marker into a 409: {denied}"
        );

        // The configured row is intact — the registration did not partially
        // land — so the listing still describes the backend that answers.
        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "coder")
            .expect("read")
            .expect("the configured row survives");
        let row: serde_json::Value = serde_json::from_slice(&raw).expect("row json");
        assert_eq!(row["source"], "config");
        assert_eq!(row["modelPath"], "openai:http://127.0.0.1:8080/v1");

        // A deregistration is refused the same way, from the same place.
        let denied = apply_guest_registry_write(&store, AI_MODELS_REGISTRY_TABLE, "coder", None)
            .expect_err("a configured alias must not be deregistered either");
        assert!(denied.contains(GUEST_REGISTRY_ALIAS_TAKEN));

        // And a free alias is unaffected: the rule guards configured rows, it
        // does not make the table read-only.
        apply_guest_registry_write(
            &store,
            AI_MODELS_REGISTRY_TABLE,
            "mine",
            Some(b"{\"alias\":\"mine\"}".to_vec()),
        )
        .expect("a free alias stays writable");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refused_registry_batch_lands_nothing_at_all() {
        let (store, dir) = temp_store();
        let configured = config_with(vec![binding(
            "coder",
            "openai:http://127.0.0.1:8080/v1",
            false,
        )]);
        publish_configured_model_bindings(&store, &configured);

        // A free alias first, then one the manifest owns. Validating per key in
        // a loop of single-key writes commits `mine` and then fails on `coder`,
        // leaving a half-applied batch — which is exactly what `batch-set`
        // promises callers it will not do.
        let entries = vec![
            ("mine".to_owned(), br#"{"alias":"mine"}"#.to_vec()),
            ("coder".to_owned(), br#"{"alias":"coder"}"#.to_vec()),
        ];
        let denied = apply_guest_registry_batch(&store, AI_MODELS_REGISTRY_TABLE, &entries)
            .expect_err("a batch touching a configured alias must be refused");
        assert!(denied.contains(GUEST_REGISTRY_ALIAS_TAKEN), "got: {denied}");

        assert!(
            store
                .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "mine")
                .expect("read")
                .is_none(),
            "the entry before the refused one must have rolled back with it"
        );
        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "coder")
            .expect("read")
            .expect("the configured row survives");
        let row: serde_json::Value = serde_json::from_slice(&raw).expect("row json");
        assert_eq!(row["source"], "config");

        // And a batch of free aliases still lands whole.
        apply_guest_registry_batch(
            &store,
            AI_MODELS_REGISTRY_TABLE,
            &[
                ("mine".to_owned(), br#"{"alias":"mine"}"#.to_vec()),
                ("yours".to_owned(), br#"{"alias":"yours"}"#.to_vec()),
            ],
        )
        .expect("free aliases stay writable");
        for alias in ["mine", "yours"] {
            assert!(
                store
                    .kv_partition_get(AI_MODELS_REGISTRY_TABLE, alias)
                    .expect("read")
                    .is_some(),
                "`{alias}` should have been written"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reload_never_withdraws_an_uploaded_row() {
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

        // The alias leaves the manifest, but the row belongs to an upload —
        // ownership is re-read inside the delete transaction.
        withdraw_changed_model_bindings(
            &store,
            &config_with(vec![binding(
                "shared",
                "openai:http://127.0.0.1:8080/v1",
                false,
            )]),
            &config_with(Vec::new()),
        );

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "shared")
            .expect("read")
            .expect("an uploaded row must survive a reload");
        let entry: serde_json::Value = serde_json::from_slice(&raw).expect("json");
        assert_eq!(entry["engine"], "gguf");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_dynamic_binding_leaves_its_uploaded_row_alone() {
        let (store, dir) = temp_store();
        // The other half of the rule: a `dynamic` alias is *not* eagerly loaded,
        // so the upload is what executes and the upload row is what should be
        // advertised. Nothing is published for it at all.
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

        publish_configured_model_bindings(&store, &config_with(vec![binding("shared", "", true)]));

        let raw = store
            .kv_partition_get(AI_MODELS_REGISTRY_TABLE, "shared")
            .expect("read")
            .expect("row");
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
        #[serde(default)]
        tool_call_parser: Option<String>,
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
            tool_call_parser: Some("qwen"),
            withdrawn: false,
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
        assert_eq!(parsed.tool_call_parser.as_deref(), Some("qwen"));

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
            value.get("toolCallParser").is_some(),
            "registry entry must serialize camelCase `toolCallParser`"
        );
        assert!(
            value.get("vram_required_mb").is_none(),
            "registry entry must not emit snake_case keys"
        );
    }

    /// The field is optional on the wire: a model that declares no dialect
    /// must produce a row with the key absent, not `null`. `guest-openai`
    /// reads it as `Option<String>` either way, but an absent key is what lets
    /// a reader predating the field ignore it entirely.
    #[test]
    fn a_model_without_a_declared_parser_omits_the_key() {
        let info = RegistryModelInfo {
            alias: "tinyllama",
            engine: "gguf",
            vram_required_mb: 0,
            status: "available",
            model_path: "/data/tachyon_data/models/tinyllama",
            source: None,
            tool_call_parser: None,
            withdrawn: false,
        };
        let value: serde_json::Value =
            serde_json::to_value(&info).expect("serialize registry entry");
        assert!(value.get("toolCallParser").is_none());
    }

    /// Upstream bindings never need their text parsed — they return structured
    /// `tool_calls` — so probing them would be both pointless and a filesystem
    /// read against a URL.
    #[test]
    fn upstream_and_mock_bindings_declare_no_parser() {
        for path in [
            "openai:https://api.example.com/v1/chat/completions",
            "  openai:https://api.example.com/v1/chat/completions",
            "mock",
            "mock:tiny",
        ] {
            assert_eq!(
                binding_tool_call_parser(path),
                None,
                "`{path}` must not be probed for a tool-call dialect"
            );
        }
    }
}
