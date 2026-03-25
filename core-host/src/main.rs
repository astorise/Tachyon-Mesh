use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{fmt, path::PathBuf, sync::Once};
use wasmtime::{Config, Engine, Linker, Module, ResourceLimiter, Store, Trap, TypedFunc};
use wasmtime_wasi::{
    p1::{self, WasiP1Ctx},
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    WasiCtxBuilder,
};

const HOST_ADDRESS: &str = "0.0.0.0:8080";
const MAX_STDOUT_BYTES: usize = 64 * 1024;
const GUEST_FUEL_BUDGET: u64 = 250_000;
const GUEST_MEMORY_LIMIT_BYTES: usize = 50 * 1024 * 1024;
const RESOURCE_LIMIT_RESPONSE: &str = "Execution trapped: Resource limit exceeded";
const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");

#[derive(Clone)]
struct AppState {
    engine: Engine,
}

struct HostState {
    wasi: WasiP1Ctx,
    limits: GuestResourceLimiter,
}

#[derive(Debug)]
struct GuestResourceLimiter {
    max_memory_bytes: usize,
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

#[derive(Serialize)]
struct IntegrityConfig<'a> {
    host_address: &'a str,
    max_stdout_bytes: usize,
    guest_fuel_budget: u64,
    guest_memory_limit_bytes: usize,
    resource_limit_response: &'a str,
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
    verify_integrity()?;

    let app = build_app(AppState {
        engine: build_engine()?,
    });

    let listener = tokio::net::TcpListener::bind(HOST_ADDRESS)
        .await
        .with_context(|| format!("failed to bind HTTP listener on {HOST_ADDRESS}"))?;

    axum::serve(listener, app)
        .await
        .context("axum server exited unexpectedly")
}

fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/*path", post(faas_handler))
        .with_state(state)
}

async fn faas_handler(
    State(state): State<AppState>,
    Path(path): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    let Some(function_name) = resolve_function_name(&path) else {
        return (
            StatusCode::NOT_FOUND,
            format!("no guest function could be resolved from `{path}`"),
        )
            .into_response();
    };

    let engine = state.engine.clone();
    let task_function_name = function_name.clone();
    let result =
        tokio::task::spawn_blocking(move || execute_guest(&engine, &task_function_name, body))
            .await;

    match result {
        Ok(Ok(stdout)) => (StatusCode::OK, stdout).into_response(),
        Ok(Err(error)) => {
            error.log_if_needed(&function_name);
            error.into_response().into_response()
        }
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest execution task failed: {error}"),
        )
            .into_response(),
    }
}

fn execute_guest(
    engine: &Engine,
    function_name: &str,
    body: Bytes,
) -> std::result::Result<Bytes, ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let module = Module::from_file(engine, &module_path).map_err(|error| {
        guest_execution_error(
            error,
            format!("failed to load guest module from {}", module_path.display()),
        )
    })?;
    let linker = build_linker(engine)?;
    let stdout = MemoryOutputPipe::new(MAX_STDOUT_BYTES);
    let wasi = WasiCtxBuilder::new()
        .stdin(MemoryInputPipe::new(body))
        .stdout(stdout.clone())
        .build_p1();
    let mut store = Store::new(engine, HostState::new(wasi));
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(GUEST_FUEL_BUDGET)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| guest_execution_error(error, "failed to instantiate guest module"))?;
    let faas_entry: TypedFunc<(), ()> =
        instance
            .get_typed_func(&mut store, "faas_entry")
            .map_err(|error| {
                guest_execution_error(error, "failed to resolve exported function `faas_entry`")
            })?;

    faas_entry
        .call(&mut store, ())
        .map_err(|error| guest_execution_error(error, "guest function `faas_entry` trapped"))?;

    Ok(split_guest_stdout(function_name, stdout.contents()))
}

fn build_linker(engine: &Engine) -> std::result::Result<Linker<HostState>, ExecutionError> {
    let mut linker = Linker::new(engine);
    p1::add_to_linker_sync(&mut linker, |state: &mut HostState| &mut state.wasi).map_err(
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

fn resolve_guest_module_path(
    function_name: &str,
) -> std::result::Result<PathBuf, GuestModuleNotFound> {
    let wasm_file = format!("{}.wasm", function_name.replace('-', "_"));
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate_strings = [
        format!("../target/wasm32-wasip1/debug/{wasm_file}"),
        format!("../target/wasm32-wasi/debug/{wasm_file}"),
        format!("target/wasm32-wasip1/debug/{wasm_file}"),
        format!("target/wasm32-wasi/debug/{wasm_file}"),
    ];
    let candidates = [
        manifest_dir.join(&candidate_strings[0]),
        manifest_dir.join(&candidate_strings[1]),
        PathBuf::from(&candidate_strings[2]),
        PathBuf::from(&candidate_strings[3]),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(normalize_path)
        .ok_or_else(|| {
            GuestModuleNotFound::new(function_name, format_candidate_list(&candidate_strings))
        })
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn format_candidate_list(paths: &[String]) -> String {
    paths.join(", ")
}

fn build_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);

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

fn verify_integrity() -> Result<()> {
    let runtime_config = canonical_runtime_config_payload()?;

    if runtime_config != EMBEDDED_CONFIG_PAYLOAD {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration does not match runtime configuration"
        ));
    }

    verify_integrity_signature(
        EMBEDDED_CONFIG_PAYLOAD,
        EMBEDDED_PUBLIC_KEY,
        EMBEDDED_SIGNATURE,
    )?;
    tracing::info!("integrity verification passed");
    Ok(())
}

fn canonical_runtime_config_payload() -> Result<String> {
    serde_json::to_string(&IntegrityConfig {
        host_address: HOST_ADDRESS,
        max_stdout_bytes: MAX_STDOUT_BYTES,
        guest_fuel_budget: GUEST_FUEL_BUDGET,
        guest_memory_limit_bytes: GUEST_MEMORY_LIMIT_BYTES,
        resource_limit_response: RESOURCE_LIMIT_RESPONSE,
    })
    .context("failed to serialize runtime integrity configuration")
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

impl HostState {
    fn new(wasi: WasiP1Ctx) -> Self {
        Self {
            wasi,
            limits: GuestResourceLimiter::new(GUEST_MEMORY_LIMIT_BYTES),
        }
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
            "guest module not found for `{}`; expected one of: {}",
            self.function_name, self.candidate_paths
        )
    }
}

impl std::error::Error for GuestModuleNotFound {}

impl ExecutionError {
    fn into_response(self) -> (StatusCode, String) {
        match self {
            Self::GuestModuleNotFound(error) => (StatusCode::NOT_FOUND, error.to_string()),
            Self::ResourceLimitExceeded { .. } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                RESOURCE_LIMIT_RESPONSE.to_string(),
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
        let payload = canonical_runtime_config_payload().expect("payload should serialize");
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
        let payload = canonical_runtime_config_payload().expect("payload should serialize");
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
    fn execute_guest_returns_stdout_payload() {
        let engine = build_engine().expect("engine should be created");
        let response = execute_guest(&engine, "guest-example", Bytes::from("Hello Lean FaaS!"))
            .expect("guest execution should succeed");

        assert_eq!(
            String::from_utf8_lossy(&response).trim(),
            "FaaS received: Hello Lean FaaS!"
        );
    }

    #[tokio::test]
    async fn router_returns_guest_stdout_for_post_request() {
        let app = build_app(AppState {
            engine: build_engine().expect("engine should be created"),
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

    #[test]
    fn guest_resource_limiter_rejects_memory_growth_past_ceiling() {
        let mut limiter = GuestResourceLimiter::new(GUEST_MEMORY_LIMIT_BYTES);
        let error = limiter
            .memory_growing(
                GUEST_MEMORY_LIMIT_BYTES,
                GUEST_MEMORY_LIMIT_BYTES + 64 * 1024,
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
    fn error_response_normalizes_resource_limit_failures() {
        let response = ExecutionError::ResourceLimitExceeded {
            kind: ResourceLimitKind::Memory,
            detail: "guest exceeded its memory quota".to_string(),
        }
        .into_response();

        assert_eq!(
            response,
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                RESOURCE_LIMIT_RESPONSE.to_string(),
            )
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
}
