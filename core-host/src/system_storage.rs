use anyhow::{anyhow, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use std::{
    fs,
    path::{Path, PathBuf},
};
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Engine, Store,
};
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
}

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
        invoke_storage_component(&engine, module_name, &root_dir, component_request)
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
    request: ComponentRequest,
) -> Result<ComponentResponse> {
    fs::create_dir_all(root_dir).with_context(|| {
        format!(
            "failed to initialize storage component root directory `{}`",
            root_dir.display()
        )
    })?;
    let module_path = crate::resolve_guest_module_path(module_name)
        .map_err(|error| anyhow!(error.to_string()))?;
    let component = Component::from_file(engine, &module_path).map_err(|error| {
        anyhow!(
            "failed to load storage component `{module_name}` from `{}`: {error}",
            module_path.display()
        )
    })?;

    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        anyhow!("failed to add WASI preview2 functions to storage component linker: {error}")
    })?;

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
