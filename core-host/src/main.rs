use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Once,
    time::Instant,
};
use telemetry::{TelemetryEvent, TelemetrySender};
use uuid::Uuid;
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Config, Engine, Instance, Linker as ModuleLinker, Module, ResourceLimiter, Store, Trap,
    TypedFunc,
};
use wasmtime_wasi::{
    p1::{self, WasiP1Ctx},
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    DirPerms, FilePerms, I32Exit, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};

mod telemetry;

mod component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "faas-guest",
    });
}

#[cfg(test)]
const DEFAULT_HOST_ADDRESS: &str = "0.0.0.0:8080";
#[cfg(test)]
const DEFAULT_MAX_STDOUT_BYTES: usize = 64 * 1024;
#[cfg(test)]
const DEFAULT_GUEST_FUEL_BUDGET: u64 = 500_000_000;
#[cfg(test)]
const DEFAULT_GUEST_MEMORY_LIMIT_BYTES: usize = 50 * 1024 * 1024;
#[cfg(test)]
const DEFAULT_RESOURCE_LIMIT_RESPONSE: &str = "Execution trapped: Resource limit exceeded";
#[cfg(test)]
const DEFAULT_ROUTE: &str = "/api/guest-example";
const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");

#[derive(Clone)]
struct AppState {
    engine: Engine,
    config: IntegrityConfig,
    http_client: Client,
    telemetry: TelemetrySender,
}

struct LegacyHostState {
    wasi: WasiP1Ctx,
    limits: GuestResourceLimiter,
}

struct ComponentHostState {
    ctx: WasiCtx,
    table: ResourceTable,
    limits: GuestResourceLimiter,
}

#[derive(Clone)]
struct GuestTelemetryContext {
    sender: TelemetrySender,
    trace_id: String,
}

#[derive(Debug)]
struct GuestResourceLimiter {
    max_memory_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestRequest {
    method: String,
    uri: String,
    body: Bytes,
}

#[derive(Debug, PartialEq, Eq)]
struct GuestHttpResponse {
    status: StatusCode,
    body: Bytes,
}

#[derive(Debug, PartialEq, Eq)]
enum GuestExecutionOutput {
    Http(GuestHttpResponse),
    LegacyStdout(Bytes),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceLimitKind {
    Fuel,
    Memory,
}

#[derive(Debug)]
struct ResourceLimitTrap {
    kind: ResourceLimitKind,
}

#[derive(Debug)]
struct GuestModuleNotFound {
    function_name: String,
    candidate_paths: String,
}

#[derive(Debug, Deserialize)]
struct GuestLogRecord {
    level: String,
    target: Option<String>,
    fields: Map<String, Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityConfig {
    host_address: String,
    max_stdout_bytes: usize,
    guest_fuel_budget: u64,
    guest_memory_limit_bytes: usize,
    resource_limit_response: String,
    routes: Vec<String>,
}

#[derive(Debug)]
enum ExecutionError {
    GuestModuleNotFound(GuestModuleNotFound),
    ResourceLimitExceeded {
        kind: ResourceLimitKind,
        detail: String,
    },
    Internal(String),
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    init_host_tracing();
    let config = verify_integrity()?;
    let telemetry = telemetry::init_telemetry();

    let app = build_app(AppState {
        engine: build_engine()?,
        config: config.clone(),
        http_client: Client::new(),
        telemetry,
    });

    let listener = tokio::net::TcpListener::bind(&config.host_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind HTTP listener on {}",
                config.host_address.as_str()
            )
        })?;

    axum::serve(listener, app)
        .await
        .context("axum server exited unexpectedly")
}

fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/*path", any(faas_handler))
        .with_state(state)
}

async fn faas_handler(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    let normalized_path = normalize_route_path(uri.path());
    let trace_id = Uuid::new_v4().to_string();
    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestStart {
            trace_id: trace_id.clone(),
            path: normalized_path.clone(),
            timestamp: Instant::now(),
        },
    );

    let response: Response = if !state.config.allows_route(&normalized_path) {
        (
            StatusCode::NOT_FOUND,
            format!("route `{normalized_path}` is not sealed in `integrity.lock`"),
        )
            .into_response()
    } else {
        match resolve_function_name(&normalized_path) {
            Some(function_name) => {
                let engine = state.engine.clone();
                let config = state.config.clone();
                let telemetry_context = GuestTelemetryContext {
                    sender: state.telemetry.clone(),
                    trace_id: trace_id.clone(),
                };
                let task_function_name = function_name.clone();
                let guest_request = GuestRequest {
                    method: method.to_string(),
                    uri: uri.to_string(),
                    body,
                };
                let result = tokio::task::spawn_blocking(move || {
                    execute_guest(
                        &engine,
                        &task_function_name,
                        guest_request,
                        &config,
                        Some(telemetry_context),
                    )
                })
                .await;

                match result {
                    Ok(Ok(output)) => {
                        let (status, body) = match output {
                            GuestExecutionOutput::Http(response) => {
                                (response.status, response.body)
                            }
                            GuestExecutionOutput::LegacyStdout(stdout) => (StatusCode::OK, stdout),
                        };

                        match resolve_mesh_response(&state.http_client, body).await {
                            Ok(response_body) => (status, response_body).into_response(),
                            Err(error) => (StatusCode::BAD_GATEWAY, error).into_response(),
                        }
                    }
                    Ok(Err(error)) => {
                        error.log_if_needed(&function_name);
                        error.into_response(&state.config).into_response()
                    }
                    Err(error) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("guest execution task failed: {error}"),
                    )
                        .into_response(),
                }
            }
            None => (
                StatusCode::NOT_FOUND,
                format!("no guest function could be resolved from `{normalized_path}`"),
            )
                .into_response(),
        }
    };

    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestEnd {
            trace_id,
            status: response.status().as_u16(),
            timestamp: Instant::now(),
        },
    );

    response
}

async fn resolve_mesh_response(
    http_client: &Client,
    body: Bytes,
) -> std::result::Result<Bytes, String> {
    let Some(url) = extract_mesh_fetch_url(&body) else {
        return Ok(body);
    };

    let response = http_client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("mesh fetch to `{url}` failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("mesh fetch to `{url}` returned an error status: {error}"))?;

    response
        .bytes()
        .await
        .map_err(|error| format!("failed to read mesh fetch response body from `{url}`: {error}"))
}

fn extract_mesh_fetch_url(stdout: &Bytes) -> Option<&str> {
    std::str::from_utf8(stdout)
        .ok()?
        .trim()
        .strip_prefix("MESH_FETCH:")
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

fn execute_guest(
    engine: &Engine,
    function_name: &str,
    request: GuestRequest,
    config: &IntegrityConfig,
    telemetry: Option<GuestTelemetryContext>,
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;

    if let Ok(component) = Component::from_file(engine, &module_path) {
        return execute_component_guest(
            engine,
            request,
            config,
            &module_path,
            &component,
            telemetry.as_ref(),
        );
    }

    let module = Module::from_file(engine, &module_path).map_err(|error| {
        guest_execution_error(
            error,
            format!(
                "failed to load guest artifact from {}",
                module_path.display()
            ),
        )
    })?;

    execute_legacy_guest(
        engine,
        function_name,
        request.body,
        config,
        &module_path,
        module,
        telemetry.as_ref(),
    )
}

fn execute_component_guest(
    engine: &Engine,
    request: GuestRequest,
    config: &IntegrityConfig,
    component_path: &Path,
    component: &Component,
    telemetry: Option<&GuestTelemetryContext>,
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to component linker",
        )
    })?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(config.guest_memory_limit_bytes),
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(config.guest_fuel_budget)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;

    let bindings = component_bindings::FaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate guest component from {}",
                    component_path.display()
                ),
            )
        })?;
    record_wasm_start(telemetry);
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            body: request.body.to_vec(),
        },
    );
    record_wasm_end(telemetry);
    let response = response.map_err(|error| {
        guest_execution_error(error, "guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutput::Http(GuestHttpResponse {
        status,
        body: Bytes::from(response.body),
    }))
}

fn execute_legacy_guest(
    engine: &Engine,
    function_name: &str,
    body: Bytes,
    config: &IntegrityConfig,
    module_path: &Path,
    module: Module,
    telemetry: Option<&GuestTelemetryContext>,
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
    let linker = build_linker(engine)?;
    let stdout = MemoryOutputPipe::new(config.max_stdout_bytes);
    let mut wasi = WasiCtxBuilder::new();
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(MemoryInputPipe::new(body))
        .stdout(stdout.clone());

    if let Some(module_dir) = module_path.parent() {
        wasi.preopened_dir(module_dir, ".", DirPerms::READ, FilePerms::READ)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to preopen guest module directory {}",
                        module_dir.display()
                    ),
                )
            })?;
    }

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(wasi, config.guest_memory_limit_bytes),
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(config.guest_fuel_budget)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| guest_execution_error(error, "failed to instantiate guest module"))?;
    let (entrypoint_name, entrypoint) =
        resolve_guest_entrypoint(&mut store, &instance).map_err(|error| {
            guest_execution_error(
                error,
                "failed to resolve exported function `faas_entry` or `_start`",
            )
        })?;

    record_wasm_start(telemetry);
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(telemetry);
    handle_guest_entrypoint_result(entrypoint_name, call_result)?;

    Ok(GuestExecutionOutput::LegacyStdout(split_guest_stdout(
        function_name,
        stdout.contents(),
    )))
}

fn legacy_guest_program_name(module_path: &Path) -> String {
    module_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("./{name}"))
        .unwrap_or_else(|| "./guest.wasm".to_owned())
}

fn resolve_guest_entrypoint(
    store: &mut Store<LegacyHostState>,
    instance: &Instance,
) -> std::result::Result<(&'static str, TypedFunc<(), ()>), wasmtime::Error> {
    match instance.get_typed_func(&mut *store, "faas_entry") {
        Ok(entrypoint) => Ok(("faas_entry", entrypoint)),
        Err(_) => instance
            .get_typed_func(&mut *store, "_start")
            .map(|entrypoint| ("_start", entrypoint)),
    }
}

fn build_linker(
    engine: &Engine,
) -> std::result::Result<ModuleLinker<LegacyHostState>, ExecutionError> {
    let mut linker = ModuleLinker::new(engine);
    p1::add_to_linker_sync(&mut linker, |state: &mut LegacyHostState| &mut state.wasi).map_err(
        |error| guest_execution_error(error, "failed to add WASI preview1 functions to linker"),
    )?;
    Ok(linker)
}

fn resolve_function_name(path: &str) -> Option<String> {
    path.split('/')
        .rev()
        .find(|segment| !segment.is_empty() && *segment != "api")
        .map(ToOwned::to_owned)
}

fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim();
    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    let normalized = with_leading_slash.trim_end_matches('/');

    if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized.to_owned()
    }
}

fn resolve_guest_module_path(
    function_name: &str,
) -> std::result::Result<PathBuf, GuestModuleNotFound> {
    let candidates = guest_module_candidate_paths(function_name);
    let candidate_strings = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(normalize_path)
        .ok_or_else(|| {
            GuestModuleNotFound::new(function_name, format_candidate_list(&candidate_strings))
        })
}

fn guest_module_candidate_paths(function_name: &str) -> Vec<PathBuf> {
    let wasm_file = format!("{}.wasm", function_name.replace('-', "_"));
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_relative_candidates = [
        format!("../target/wasm32-wasip2/debug/{wasm_file}"),
        format!("../target/wasm32-wasip2/release/{wasm_file}"),
        format!("../target/wasm32-wasip1/debug/{wasm_file}"),
        format!("../target/wasm32-wasip1/release/{wasm_file}"),
        format!("../target/wasm32-wasi/debug/{wasm_file}"),
        format!("../target/wasm32-wasi/release/{wasm_file}"),
    ];
    let workspace_relative_candidates = [
        format!("target/wasm32-wasip2/debug/{wasm_file}"),
        format!("target/wasm32-wasip2/release/{wasm_file}"),
        format!("target/wasm32-wasip1/debug/{wasm_file}"),
        format!("target/wasm32-wasip1/release/{wasm_file}"),
        format!("target/wasm32-wasi/debug/{wasm_file}"),
        format!("target/wasm32-wasi/release/{wasm_file}"),
        format!("guest-modules/{wasm_file}"),
    ];

    manifest_relative_candidates
        .into_iter()
        .map(|path| manifest_dir.join(path))
        .chain(workspace_relative_candidates.into_iter().map(PathBuf::from))
        .chain(std::iter::once(PathBuf::from(format!(
            "/app/guest-modules/{wasm_file}"
        ))))
        .collect()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn format_candidate_list(paths: &[String]) -> String {
    paths.join(", ")
}

fn record_wasm_start(telemetry: Option<&GuestTelemetryContext>) {
    record_wasm_event(telemetry, true);
}

fn record_wasm_end(telemetry: Option<&GuestTelemetryContext>) {
    record_wasm_event(telemetry, false);
}

fn record_wasm_event(telemetry: Option<&GuestTelemetryContext>, is_start: bool) {
    let Some(telemetry) = telemetry else {
        return;
    };

    let event = if is_start {
        TelemetryEvent::WasmStart {
            trace_id: telemetry.trace_id.clone(),
            timestamp: Instant::now(),
        }
    } else {
        TelemetryEvent::WasmEnd {
            trace_id: telemetry.trace_id.clone(),
            timestamp: Instant::now(),
        }
    };

    telemetry::record_event(&telemetry.sender, event);
}

fn handle_guest_entrypoint_result(
    entrypoint_name: &str,
    result: std::result::Result<(), wasmtime::Error>,
) -> std::result::Result<(), ExecutionError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if entrypoint_name == "_start" => match error.downcast_ref::<I32Exit>() {
            Some(exit) => {
                if exit.0 != 0 {
                    tracing::warn!(
                        guest_entrypoint = entrypoint_name,
                        exit_status = exit.0,
                        "command-style WASI guest exited non-zero; preserving stdout response"
                    );
                }
                Ok(())
            }
            None => Err(guest_execution_error(
                error,
                format!("guest function `{entrypoint_name}` trapped"),
            )),
        },
        Err(error) => Err(guest_execution_error(
            error,
            format!("guest function `{entrypoint_name}` trapped"),
        )),
    }
}

fn build_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.wasm_component_model(true);

    Engine::new(&config).map_err(|error| {
        anyhow!("failed to create Wasmtime engine with fuel metering enabled: {error}")
    })
}

fn init_host_tracing() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(true)
            .try_init();
    });
}

fn verify_integrity() -> Result<IntegrityConfig> {
    verify_integrity_signature(
        EMBEDDED_CONFIG_PAYLOAD,
        EMBEDDED_PUBLIC_KEY,
        EMBEDDED_SIGNATURE,
    )?;
    let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
        .context("failed to parse embedded sealed configuration")?;
    let config = validate_integrity_config(config)?;
    tracing::info!("integrity verification passed");
    Ok(config)
}

fn validate_integrity_config(mut config: IntegrityConfig) -> Result<IntegrityConfig> {
    if config.host_address.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `host_address`"
        ));
    }

    if config.max_stdout_bytes == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `max_stdout_bytes`"
        ));
    }

    if config.guest_fuel_budget == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `guest_fuel_budget`"
        ));
    }

    if config.guest_memory_limit_bytes == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `guest_memory_limit_bytes`"
        ));
    }

    if config.resource_limit_response.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `resource_limit_response`"
        ));
    }

    config.routes = normalize_config_routes(config.routes)?;
    Ok(config)
}

fn normalize_config_routes(routes: Vec<String>) -> Result<Vec<String>> {
    if routes.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must define at least one route"
        ));
    }

    let mut normalized = routes
        .into_iter()
        .map(|route| validate_route_path(&route))
        .collect::<Result<Vec<_>>>()?;

    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

fn validate_route_path(path: &str) -> Result<String> {
    let normalized = normalize_route_path(path);

    if normalized == "/" || resolve_function_name(&normalized).is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` does not resolve to a guest function"
        ));
    }

    Ok(normalized)
}

#[cfg(test)]
fn canonical_config_payload(config: &IntegrityConfig) -> Result<String> {
    serde_json::to_string(config).context("failed to serialize runtime integrity configuration")
}

fn verify_integrity_signature(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
) -> Result<()> {
    let payload_hash = Sha256::digest(payload.as_bytes());
    let public_key_bytes = decode_hex_array::<32>(public_key_hex, "public key")?;
    let signature_bytes = decode_hex_array::<64>(signature_hex, "signature")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .context("invalid embedded Ed25519 public key")?;
    let signature = Signature::from_bytes(&signature_bytes);

    verifying_key
        .verify(&payload_hash, &signature)
        .map_err(|error| {
            anyhow!("Integrity Validation Failed: signature verification failed: {error}")
        })
}

fn decode_hex_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let decoded =
        hex::decode(value).with_context(|| format!("failed to decode embedded {label} as hex"))?;

    decoded
        .try_into()
        .map_err(|_| anyhow!("embedded {label} has an unexpected byte length"))
}

fn guest_execution_error(error: wasmtime::Error, context: impl Into<String>) -> ExecutionError {
    let error = error.context(context.into());

    if let Some(kind) = classify_resource_limit(&error) {
        return ExecutionError::ResourceLimitExceeded {
            kind,
            detail: format!("{error:#}"),
        };
    }

    ExecutionError::Internal(format!("{error:#}"))
}

fn classify_resource_limit(error: &wasmtime::Error) -> Option<ResourceLimitKind> {
    if let Some(limit) = error.downcast_ref::<ResourceLimitTrap>() {
        return Some(limit.kind);
    }

    error.downcast_ref::<Trap>().and_then(|trap| match trap {
        Trap::OutOfFuel => Some(ResourceLimitKind::Fuel),
        Trap::AllocationTooLarge => Some(ResourceLimitKind::Memory),
        _ => None,
    })
}

fn split_guest_stdout(function_name: &str, stdout: Bytes) -> Bytes {
    let output = String::from_utf8_lossy(&stdout);
    let mut response = String::new();

    for segment in output.split_inclusive('\n') {
        let line = trim_line_endings(segment);

        if let Some(record) = parse_guest_log_line(line) {
            forward_guest_log(function_name, record);
            continue;
        }

        response.push_str(segment);
    }

    Bytes::from(response)
}

fn trim_line_endings(segment: &str) -> &str {
    let trimmed = segment.strip_suffix('\n').unwrap_or(segment);
    trimmed.strip_suffix('\r').unwrap_or(trimmed)
}

fn parse_guest_log_line(line: &str) -> Option<GuestLogRecord> {
    serde_json::from_str::<GuestLogRecord>(line).ok()
}

fn forward_guest_log(function_name: &str, record: GuestLogRecord) {
    let level = record.level.to_ascii_uppercase();
    let target = record.target.unwrap_or_else(|| "guest".to_owned());
    let message = record
        .fields
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("guest emitted a structured log")
        .to_owned();
    let fields = Value::Object(record.fields).to_string();

    match level.as_str() {
        "TRACE" => tracing::trace!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "DEBUG" => tracing::debug!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "WARN" => tracing::warn!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        "ERROR" => tracing::error!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
        _ => tracing::info!(
            guest_function = function_name,
            guest_target = %target,
            guest_fields = %fields,
            "{message}"
        ),
    }
}

impl LegacyHostState {
    fn new(wasi: WasiP1Ctx, max_memory_bytes: usize) -> Self {
        Self {
            wasi,
            limits: GuestResourceLimiter::new(max_memory_bytes),
        }
    }
}

impl ComponentHostState {
    fn new(max_memory_bytes: usize) -> Self {
        let mut wasi = WasiCtxBuilder::new();
        Self {
            ctx: wasi.build(),
            table: ResourceTable::new(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
        }
    }
}

impl WasiView for ComponentHostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl IntegrityConfig {
    #[cfg(test)]
    fn default_sealed() -> Self {
        Self {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            routes: vec![DEFAULT_ROUTE.to_owned()],
        }
    }

    fn allows_route(&self, path: &str) -> bool {
        let normalized = normalize_route_path(path);
        self.routes.iter().any(|route| route == &normalized)
    }
}

impl GuestResourceLimiter {
    fn new(max_memory_bytes: usize) -> Self {
        Self { max_memory_bytes }
    }
}

impl ResourceLimiter for GuestResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.max_memory_bytes {
            return Err(ResourceLimitTrap {
                kind: ResourceLimitKind::Memory,
            }
            .into());
        }

        Ok(maximum.is_none_or(|max| desired <= max))
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(maximum.is_none_or(|max| desired <= max))
    }
}

impl fmt::Display for ResourceLimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fuel => f.write_str("fuel"),
            Self::Memory => f.write_str("memory"),
        }
    }
}

impl fmt::Display for ResourceLimitTrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "guest exceeded its {} quota", self.kind)
    }
}

impl std::error::Error for ResourceLimitTrap {}

impl GuestModuleNotFound {
    fn new(function_name: &str, candidate_paths: String) -> Self {
        Self {
            function_name: function_name.to_owned(),
            candidate_paths,
        }
    }
}

impl fmt::Display for GuestModuleNotFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "guest artifact not found for `{}`; expected one of: {}",
            self.function_name, self.candidate_paths
        )
    }
}

impl std::error::Error for GuestModuleNotFound {}

impl ExecutionError {
    fn into_response(self, config: &IntegrityConfig) -> (StatusCode, String) {
        match self {
            Self::GuestModuleNotFound(error) => (StatusCode::NOT_FOUND, error.to_string()),
            Self::ResourceLimitExceeded { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                config.resource_limit_response.clone(),
            ),
            Self::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        }
    }

    fn log_if_needed(&self, function_name: &str) {
        if let Self::ResourceLimitExceeded { kind, detail } = self {
            eprintln!("warning: guest `{function_name}` exceeded its {kind} quota: {detail}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use ed25519_dalek::{Signer, SigningKey};
    use http_body_util::BodyExt;
    use tower::util::ServiceExt;

    #[test]
    fn split_guest_stdout_removes_json_log_lines() {
        let stdout = Bytes::from(
            "{\"level\":\"INFO\",\"target\":\"guest_example\",\"fields\":{\"message\":\"guest-example received a request payload\"}}\nFaaS received: Hello Lean FaaS!\n",
        );

        let response = split_guest_stdout("guest-example", stdout);

        assert_eq!(
            String::from_utf8_lossy(&response),
            "FaaS received: Hello Lean FaaS!\n"
        );
    }

    #[test]
    fn verify_integrity_signature_accepts_valid_material() {
        let payload = canonical_config_payload(&IntegrityConfig::default_sealed())
            .expect("payload should serialize");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));

        verify_integrity_signature(
            &payload,
            &hex::encode(signing_key.verifying_key().to_bytes()),
            &hex::encode(signature.to_bytes()),
        )
        .expect("signature should verify");
    }

    #[test]
    fn verify_integrity_signature_rejects_tampered_payload() {
        let payload = canonical_config_payload(&IntegrityConfig::default_sealed())
            .expect("payload should serialize");
        let signing_key = SigningKey::from_bytes(&[9_u8; 32]);
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));

        let error = verify_integrity_signature(
            "{\"tampered\":true}",
            &hex::encode(signing_key.verifying_key().to_bytes()),
            &hex::encode(signature.to_bytes()),
        )
        .expect_err("tampered payload should fail verification");

        assert!(
            error.to_string().contains("Integrity Validation Failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn execute_guest_returns_component_response_payload() {
        let engine = build_engine().expect("engine should be created");
        let config = IntegrityConfig::default_sealed();
        let response = execute_guest(
            &engine,
            "guest-example",
            GuestRequest {
                method: "POST".to_owned(),
                uri: "/api/guest-example".to_owned(),
                body: Bytes::from("Hello Lean FaaS!"),
            },
            &config,
            None,
        )
        .expect("guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutput::Http(GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from("FaaS received: Hello Lean FaaS!"),
            })
        );
    }

    #[test]
    fn execute_guest_falls_back_to_legacy_stdout_for_non_component_module() {
        let engine = build_engine().expect("engine should be created");
        let config = IntegrityConfig::default_sealed();
        let response = execute_guest(
            &engine,
            "guest-call-legacy",
            GuestRequest {
                method: "GET".to_owned(),
                uri: "/api/guest-call-legacy".to_owned(),
                body: Bytes::new(),
            },
            &config,
            None,
        )
        .expect("legacy guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutput::LegacyStdout(Bytes::from(
                "MESH_FETCH:http://legacy-service:8081/ping\n"
            ))
        );
    }

    #[tokio::test]
    async fn router_returns_guest_stdout_for_post_request() {
        let app = build_app(AppState {
            engine: build_engine().expect("engine should be created"),
            config: IntegrityConfig::default_sealed(),
            http_client: Client::new(),
            telemetry: telemetry::init_test_telemetry(),
        });
        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("Hello Lean FaaS!"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();

        assert_eq!(
            String::from_utf8_lossy(&body).trim(),
            "FaaS received: Hello Lean FaaS!"
        );
    }

    #[tokio::test]
    async fn router_accepts_get_requests() {
        let app = build_app(AppState {
            engine: build_engine().expect("engine should be created"),
            config: IntegrityConfig::default_sealed(),
            http_client: Client::new(),
            telemetry: telemetry::init_test_telemetry(),
        });
        let response = app
            .oneshot(
                Request::get("/api/guest-example")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();

        assert_eq!(
            String::from_utf8_lossy(&body).trim(),
            "FaaS received an empty payload"
        );
    }

    #[tokio::test]
    async fn router_rejects_unsealed_routes() {
        let app = build_app(AppState {
            engine: build_engine().expect("engine should be created"),
            config: IntegrityConfig::default_sealed(),
            http_client: Client::new(),
            telemetry: telemetry::init_test_telemetry(),
        });
        let response = app
            .oneshot(
                Request::post("/api/guest-malicious")
                    .body(Body::from("blocked"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn router_emits_async_telemetry_metrics() {
        use serde_json::Value;
        use std::{
            sync::{Arc, Mutex},
            time::Duration,
        };

        let captured = Arc::new(Mutex::new(Vec::new()));
        let telemetry = telemetry::init_test_telemetry_with_emitter({
            let captured = Arc::clone(&captured);
            move |line| {
                captured
                    .lock()
                    .expect("captured telemetry should not be poisoned")
                    .push(line);
            }
        });
        let app = build_app(AppState {
            engine: build_engine().expect("engine should be created"),
            config: IntegrityConfig::default_sealed(),
            http_client: Client::new(),
            telemetry,
        });

        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("Hello Lean FaaS!"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);

        let line = tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Some(line) = captured
                    .lock()
                    .expect("captured telemetry should not be poisoned")
                    .first()
                    .cloned()
                {
                    break line;
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("telemetry line should be emitted");
        let record: Value =
            serde_json::from_str(&line).expect("telemetry output should be valid JSON");

        assert_eq!(record["path"], "/api/guest-example");
        assert_eq!(record["status"], 200);
        assert!(record["trace_id"].as_str().is_some());
        assert!(record["total_duration_us"].as_u64().is_some());
        assert!(record["wasm_duration_us"].as_u64().is_some());
        assert!(record["host_overhead_us"].as_u64().is_some());
    }

    #[test]
    fn guest_resource_limiter_rejects_memory_growth_past_ceiling() {
        let config = IntegrityConfig::default_sealed();
        let mut limiter = GuestResourceLimiter::new(config.guest_memory_limit_bytes);
        let error = limiter
            .memory_growing(
                config.guest_memory_limit_bytes,
                config.guest_memory_limit_bytes + 64 * 1024,
                None,
            )
            .expect_err("growth past the quota should fail");

        assert_eq!(
            error
                .downcast_ref::<ResourceLimitTrap>()
                .map(|error| error.kind),
            Some(ResourceLimitKind::Memory)
        );
    }

    #[test]
    fn extract_mesh_fetch_url_recognizes_bridge_command() {
        let stdout = Bytes::from("MESH_FETCH:http://legacy-service:8081/ping\n");

        assert_eq!(
            extract_mesh_fetch_url(&stdout),
            Some("http://legacy-service:8081/ping")
        );
    }

    #[test]
    fn extract_mesh_fetch_url_ignores_regular_guest_output() {
        let stdout = Bytes::from("FaaS received: Hello Lean FaaS!\n");

        assert_eq!(extract_mesh_fetch_url(&stdout), None);
    }

    #[test]
    fn error_response_normalizes_resource_limit_failures() {
        let config = IntegrityConfig::default_sealed();
        let response = ExecutionError::ResourceLimitExceeded {
            kind: ResourceLimitKind::Memory,
            detail: "guest exceeded its memory quota".to_string(),
        }
        .into_response(&config);

        assert_eq!(
            response,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                config.resource_limit_response,
            )
        );
    }

    #[test]
    fn validate_integrity_config_normalizes_routes() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![
            "api/guest-example".to_owned(),
            "/api/guest-example/".to_owned(),
            "/api/guest-malicious".to_owned(),
        ];

        let config = validate_integrity_config(config).expect("config should validate");

        assert_eq!(
            config.routes,
            vec![
                "/api/guest-example".to_owned(),
                "/api/guest-malicious".to_owned(),
            ]
        );
    }

    #[test]
    fn embedded_integrity_payload_is_a_valid_runtime_config() {
        let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
            .expect("embedded payload should deserialize into an integrity config");
        let config = validate_integrity_config(config).expect("embedded config should validate");

        assert_eq!(config.guest_fuel_budget, DEFAULT_GUEST_FUEL_BUDGET);
        assert!(config.allows_route("/api/guest-example"));
        assert!(config.allows_route("/api/guest-csharp"));
        assert!(config.allows_route("/api/guest-java"));
    }

    #[test]
    fn guest_module_candidates_cover_release_and_container_paths() {
        let candidates = guest_module_candidate_paths("guest-example")
            .into_iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();

        assert!(candidates.iter().any(|path| {
            path.ends_with("/target/wasm32-wasip2/release/guest_example.wasm")
                || path == "target/wasm32-wasip2/release/guest_example.wasm"
        }));
        assert!(candidates.iter().any(|path| {
            path.ends_with("/target/wasm32-wasip1/release/guest_example.wasm")
                || path == "target/wasm32-wasip1/release/guest_example.wasm"
        }));
        assert!(candidates
            .iter()
            .any(|path| path.ends_with("guest-modules/guest_example.wasm")));
    }

    #[test]
    fn guest_module_candidates_normalize_hyphenated_names_to_underscores() {
        let candidates = guest_module_candidate_paths("guest-csharp")
            .into_iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>();

        assert!(candidates
            .iter()
            .any(|path| path.ends_with("guest-modules/guest_csharp.wasm")));
    }

    #[test]
    fn legacy_guest_program_name_is_a_guest_visible_relative_path() {
        let program_name =
            legacy_guest_program_name(Path::new("/app/guest-modules/guest_csharp.wasm"));

        assert_eq!(program_name, "./guest_csharp.wasm");
    }

    #[test]
    fn resolve_function_name_supports_hyphenated_guest_routes() {
        assert_eq!(
            resolve_function_name("/api/guest-java"),
            Some("guest-java".to_owned())
        );
    }

    #[test]
    fn classify_resource_limit_detects_fuel_traps() {
        let error: wasmtime::Error = Trap::OutOfFuel.into();

        assert_eq!(
            classify_resource_limit(&error),
            Some(ResourceLimitKind::Fuel)
        );
    }

    #[test]
    fn zero_exit_status_from_command_guest_is_treated_as_success() {
        let result = handle_guest_entrypoint_result("_start", Err(I32Exit(0).into()));

        assert!(result.is_ok());
    }

    #[test]
    fn nonzero_exit_status_from_start_guest_is_preserved_as_success() {
        let result = handle_guest_entrypoint_result("_start", Err(I32Exit(1).into()));

        assert!(result.is_ok());
    }

    #[test]
    fn nonzero_exit_status_from_faas_entry_remains_an_error() {
        let error = handle_guest_entrypoint_result("faas_entry", Err(I32Exit(1).into()))
            .expect_err("non-zero faas_entry exit should fail");

        match error {
            ExecutionError::Internal(message) => {
                assert!(message.contains("Exited with i32 exit status 1"));
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }
}
