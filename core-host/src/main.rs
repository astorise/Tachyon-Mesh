use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
#[cfg(feature = "websockets")]
use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
use axum::{
    body::{Body, Bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri},
    middleware::from_fn,
    response::{IntoResponse, Response},
    Extension, Router,
};
use clap::{Args as ClapArgs, Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
#[cfg(feature = "websockets")]
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use hyper::body::{Frame, SizeHint};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
use rand::Rng;
use reqwest::Client;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fmt, fs,
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Once,
    },
    task::{Context as TaskContext, Poll},
    time::{Duration, Instant, SystemTime},
};
use telemetry::{TelemetryEvent, TelemetryHandle, TelemetrySnapshot};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio_rustls::LazyConfigAcceptor;
use uuid::Uuid;
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Config, Engine, Instance, Linker as ModuleLinker, Module, PoolingAllocationConfig,
    ResourceLimiter, Store, Trap, TypedFunc,
};
#[cfg(test)]
use wasmtime_wasi::cli::OutputFile;
use wasmtime_wasi::{
    cli::{InputFile, IsTerminal, StdinStream, StdoutStream},
    p1::{self, WasiP1Ctx},
    p2::{InputStream, OutputStream, Pollable, StreamError, StreamResult},
    DirPerms, FilePerms, I32Exit, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};
#[cfg(feature = "ai-inference")]
use wasmtime_wasi_nn::witx::WasiNnCtx;

#[cfg(feature = "ai-inference")]
mod ai_inference;
#[cfg(feature = "rate-limit")]
mod rate_limit;
#[cfg(feature = "resiliency")]
mod resiliency;
#[cfg(feature = "http3")]
mod server_h3;
mod store;
mod telemetry;
mod tls_runtime;

mod component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "faas-guest",
    });
}

mod system_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "system-faas-guest",
    });
}

mod udp_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "udp-faas-guest",
    });
}

#[cfg(feature = "websockets")]
mod websocket_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "websocket-faas-guest",
    });
}

mod background_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "background-system-faas",
    });
}

#[cfg(feature = "ai-inference")]
mod accelerator_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit-accelerator",
        world: "host",
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
#[cfg(test)]
const DEFAULT_SYSTEM_ROUTE: &str = "/metrics";
#[cfg(test)]
const DEFAULT_TLS_ADDRESS: &str = "127.0.0.1:3443";
const ACME_STAGING_MOCK_MODE: &str = "ACME_STAGING_MOCK";
const CERT_MANAGER_GUEST_CERT_DIR: &str = "/app/certs";
const SYSTEM_METERING_ROUTE: &str = "/system/metering";
const SYSTEM_CERT_MANAGER_ROUTE: &str = "/system/cert-manager";
const SYSTEM_LOGGER_ROUTE: &str = "/system/logger";
const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");
const INTEGRITY_MANIFEST_PATH_ENV: &str = "TACHYON_INTEGRITY_MANIFEST";
const DEFAULT_HOP_LIMIT: u32 = 10;
const HOP_LIMIT_HEADER: &str = "x-tachyon-hop-limit";
const COHORT_HEADER: &str = "x-cohort";
const TACHYON_COHORT_HEADER: &str = "x-tachyon-cohort";
const TACHYON_IDENTITY_HEADER: &str = "x-tachyon-identity";
const TACHYON_SYSTEM_PUBLIC_KEY_ENV: &str = "TACHYON_SYSTEM_PUBLIC_KEY";
#[cfg(unix)]
const TACHYON_DISCOVERY_DIR_ENV: &str = "TACHYON_DISCOVERY_DIR";
const LOG_QUEUE_CAPACITY: usize = 64_000;
const LOG_BATCH_SIZE: usize = 1_000;
const LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD: usize = 32;
const DEFAULT_ROUTE_MAX_CONCURRENCY: u32 = 100;
const DEFAULT_ROUTE_VERSION: &str = "0.0.0";
const DEFAULT_TELEMETRY_SAMPLE_RATE: f64 = 0.0;
const AUTOSCALING_TICK_INTERVAL: Duration = Duration::from_secs(5);
const VOLUME_GC_TICK_INTERVAL: Duration = Duration::from_secs(60);
const DRAINING_REAPER_TICK_INTERVAL: Duration = Duration::from_secs(1);
const DRAINING_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);
const TELEMETRY_EXPORT_QUEUE_CAPACITY: usize = 1024;
const TELEMETRY_EXPORT_BATCH_SIZE: usize = 32;
const UDP_LAYER4_QUEUE_CAPACITY: usize = 256;
const UDP_LAYER4_MAX_WORKERS_PER_LISTENER: usize = 8;
const UDP_LAYER4_MAX_DATAGRAM_SIZE: usize = 65_507;
const BUFFER_RAM_REQUEST_CAPACITY: usize = 32;
const BUFFER_TOTAL_REQUEST_CAPACITY: usize = 256;
const BUFFER_REPLAY_RETRY_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(test))]
const BUFFER_RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
const BUFFER_RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
const PRESSURE_MONITOR_IDLE_SLEEP_INTERVAL: Duration = Duration::from_secs(60);
const PRESSURE_MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(unix)]
const PEER_PRESSURE_STALE_AFTER: Duration = Duration::from_secs(10);
const PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD: usize = 8;
const PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD: usize = 32;
const IDENTITY_TOKEN_TTL: Duration = Duration::from_secs(30);
const IDENTITY_TOKEN_PREFIX: &str = "tachyon.v1";
const KUBERNETES_SERVICE_BASE_URL: &str = "https://kubernetes.default.svc";
const MOCK_K8S_URL_ENV: &str = "TACHYON_MOCK_K8S_URL";
#[cfg(unix)]
const DEFAULT_DISCOVERY_DIR: &str = "/tmp/tachyon/peers";
#[cfg(not(test))]
const ROUTE_CONCURRENCY_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const ROUTE_CONCURRENCY_WAIT_TIMEOUT: Duration = Duration::from_millis(50);
const POOLING_CORE_INSTANCES_MULTIPLIER: u32 = 8;
const POOLING_MEMORIES_MULTIPLIER: u32 = 2;
const POOLING_TABLES_MULTIPLIER: u32 = 2;
const POOLING_INSTANCE_METADATA_BYTES: usize = 1 << 20;
const POOLING_MAX_CORE_INSTANCES_PER_COMPONENT: u32 = 50;
const POOLING_MAX_MEMORIES_PER_COMPONENT: u32 = 8;
const POOLING_MAX_TABLES_PER_COMPONENT: u32 = 8;

fn default_max_concurrency() -> u32 {
    DEFAULT_ROUTE_MAX_CONCURRENCY
}

fn default_route_version() -> String {
    DEFAULT_ROUTE_VERSION.to_owned()
}

fn default_telemetry_sample_rate() -> f64 {
    DEFAULT_TELEMETRY_SAMPLE_RATE
}

fn is_default_telemetry_sample_rate(sample_rate: &f64) -> bool {
    (*sample_rate - DEFAULT_TELEMETRY_SAMPLE_RATE).abs() < f64::EPSILON
}

fn unix_timestamp_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system clock is set before the Unix epoch")?
        .as_secs())
}

fn core_store_path(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("tachyon.db")
}

fn buffered_request_spool_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("buffered-requests")
}

async fn open_core_store_for_manifest(manifest_path: &Path) -> Result<Arc<store::CoreStore>> {
    let manifest_path = manifest_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        store::CoreStore::open(&core_store_path(&manifest_path)).map(Arc::new)
    })
    .await
    .context("core store initialization task failed")?
}

fn forbidden_error(message: &str) -> String {
    format!("forbidden:{message}")
}

fn is_default_route_version(version: &String) -> bool {
    version == DEFAULT_ROUTE_VERSION
}

fn default_volume_type() -> VolumeType {
    VolumeType::Host
}

fn is_default_volume_type(volume_type: &VolumeType) -> bool {
    *volume_type == VolumeType::Host
}

fn is_default_model_device(device: &ModelDevice) -> bool {
    *device == ModelDevice::Cpu
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<ArcSwap<RuntimeState>>,
    draining_runtimes: Arc<Mutex<Vec<DrainingRuntime>>>,
    http_client: Client,
    async_log_sender: mpsc::Sender<AsyncLogEntry>,
    secrets_vault: SecretsVault,
    host_identity: Arc<HostIdentity>,
    uds_fast_path: Arc<UdsFastPathRegistry>,
    storage_broker: Arc<StorageBrokerManager>,
    core_store: Arc<store::CoreStore>,
    buffered_requests: Arc<BufferedRequestManager>,
    volume_manager: Arc<VolumeManager>,
    telemetry: TelemetryHandle,
    tls_manager: Arc<tls_runtime::TlsManager>,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    manifest_path: PathBuf,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    background_workers: Arc<BackgroundWorkerManager>,
}

#[derive(Clone)]
struct RuntimeState {
    engine: Engine,
    metered_engine: Engine,
    config: IntegrityConfig,
    route_registry: Arc<RouteRegistry>,
    #[allow(dead_code)]
    batch_target_registry: Arc<BatchTargetRegistry>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

struct DrainingRuntime {
    runtime: Arc<RuntimeState>,
    draining_since: Instant,
}

struct TcpLayer4ListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    local_addr: SocketAddr,
    join_handle: tokio::task::JoinHandle<()>,
}

struct UdpLayer4ListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    local_addr: SocketAddr,
    join_handles: Vec<tokio::task::JoinHandle<()>>,
}

struct HttpsListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    local_addr: SocketAddr,
    join_handle: tokio::task::JoinHandle<()>,
}

struct Http3ListenerHandle {
    #[allow(dead_code)]
    local_addr: SocketAddr,
    join_handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HopLimit(u32);

#[cfg(unix)]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct UdsPeerMetadata {
    host_id: String,
    ip: String,
    socket_path: String,
    protocols: Vec<String>,
    #[serde(default)]
    pressure_state: PeerPressureState,
    #[serde(default)]
    last_pressure_update_unix_ms: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveredUdsPeer {
    metadata_path: PathBuf,
    socket_path: PathBuf,
    metadata: UdsPeerMetadata,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct LocalUdsEndpoint {
    metadata_path: PathBuf,
    socket_path: PathBuf,
}

#[cfg(unix)]
#[derive(Clone, Default)]
struct UdsFastPathRegistry {
    discovery_dir_override: Arc<Mutex<Option<PathBuf>>>,
    peers: Arc<Mutex<HashMap<String, DiscoveredUdsPeer>>>,
    local_endpoint: Arc<Mutex<Option<LocalUdsEndpoint>>>,
}

#[cfg(not(unix))]
#[derive(Clone, Default)]
struct UdsFastPathRegistry;

#[cfg(unix)]
fn new_uds_fast_path_registry() -> UdsFastPathRegistry {
    UdsFastPathRegistry::default()
}

#[cfg(not(unix))]
fn new_uds_fast_path_registry() -> UdsFastPathRegistry {
    UdsFastPathRegistry
}

struct LegacyHostState {
    wasi: WasiP1Ctx,
    #[cfg(feature = "ai-inference")]
    wasi_nn: WasiNnCtx,
    limits: GuestResourceLimiter,
}

struct ComponentHostState {
    ctx: WasiCtx,
    table: ResourceTable,
    limits: GuestResourceLimiter,
    secrets: SecretAccess,
    runtime_config: IntegrityConfig,
    request_headers: HeaderMap,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
    outbound_http_client: reqwest::blocking::Client,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Option<Arc<ai_inference::AiInferenceRuntime>>,
    #[cfg(feature = "ai-inference")]
    allowed_model_aliases: BTreeSet<String>,
    #[cfg(feature = "ai-inference")]
    accelerator_models: HashMap<u32, LoadedAcceleratorModel>,
    #[cfg(feature = "ai-inference")]
    next_accelerator_model_id: u32,
}

#[cfg(feature = "ai-inference")]
#[derive(Clone, Debug)]
struct LoadedAcceleratorModel {
    alias: String,
    accelerator: ai_inference::AcceleratorKind,
}

struct BatchCommandState {
    ctx: WasiCtx,
    table: ResourceTable,
}

#[derive(Clone)]
struct GuestTelemetryContext {
    handle: TelemetryHandle,
    trace_id: String,
}

struct GuestExecutionContext {
    config: IntegrityConfig,
    sampled_execution: bool,
    runtime_telemetry: TelemetryHandle,
    async_log_sender: mpsc::Sender<AsyncLogEntry>,
    secret_access: SecretAccess,
    request_headers: HeaderMap,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    telemetry: Option<GuestTelemetryContext>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

struct BackgroundTickRunner {
    function_name: String,
    route_path: String,
    store: Store<ComponentHostState>,
    bindings: background_component_bindings::BackgroundSystemFaas,
}

#[derive(Clone, Default)]
struct SecretsVault {
    #[cfg(feature = "secrets-vault")]
    entries: Arc<HashMap<String, String>>,
}

#[cfg_attr(not(feature = "secrets-vault"), allow(dead_code))]
#[derive(Clone, Debug, Default)]
struct SecretAccess {
    allowed_secrets: BTreeSet<String>,
    #[cfg(feature = "secrets-vault")]
    entries: Arc<HashMap<String, String>>,
}

#[derive(Debug)]
struct GuestResourceLimiter {
    max_memory_bytes: usize,
}

type GuestHttpFields = Vec<(String, String)>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestRequest {
    method: String,
    uri: String,
    headers: GuestHttpFields,
    body: Bytes,
    trailers: GuestHttpFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestHttpResponse {
    status: StatusCode,
    headers: GuestHttpFields,
    body: Bytes,
    trailers: GuestHttpFields,
}

struct GuestResponseBody {
    data: Option<Bytes>,
    trailers: Option<HeaderMap>,
    _completion_guard: Option<RouteResponseGuard>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UdpResponseDatagram {
    target: SocketAddr,
    payload: Bytes,
}

#[derive(Debug, PartialEq, Eq)]
enum GuestExecutionOutput {
    Http(GuestHttpResponse),
    LegacyStdout(Bytes),
}

#[derive(Debug, PartialEq, Eq)]
struct GuestExecutionOutcome {
    output: GuestExecutionOutput,
    fuel_consumed: Option<u64>,
}

struct RouteExecutionResult {
    response: GuestHttpResponse,
    fuel_consumed: Option<u64>,
    completion_guard: Option<RouteResponseGuard>,
}

type BufferedRouteResult = std::result::Result<RouteExecutionResult, (StatusCode, String)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BufferedRequestTier {
    Ram,
    Disk,
}

#[derive(Clone)]
struct BufferedRequestManager {
    disk_dir: PathBuf,
    ram_capacity: usize,
    total_capacity: usize,
    state: Arc<Mutex<BufferedRequestState>>,
    notify: Arc<Notify>,
}

struct BufferedRequestState {
    next_id: u64,
    ram_queue: VecDeque<BufferedMemoryRequest>,
    disk_queue: VecDeque<BufferedDiskRequest>,
}

struct BufferedMemoryRequest {
    id: String,
    request: BufferedRouteRequest,
    completion: oneshot::Sender<BufferedRouteResult>,
}

struct BufferedDiskRequest {
    id: String,
    path: PathBuf,
    completion: oneshot::Sender<BufferedRouteResult>,
}

struct BufferedQueueItem {
    id: String,
    request: BufferedRouteRequest,
    completion: oneshot::Sender<BufferedRouteResult>,
    disk_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct BufferedRouteRequest {
    route_path: String,
    selected_module: String,
    method: String,
    uri: String,
    headers: GuestHttpFields,
    body: Vec<u8>,
    trailers: GuestHttpFields,
    hop_limit: u32,
    trace_id: Option<String>,
    sampled_execution: bool,
}

impl fmt::Debug for RouteExecutionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteExecutionResult")
            .field("response", &self.response)
            .field("fuel_consumed", &self.fuel_consumed)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteLifecycleState {
    Active,
    Draining,
}

struct ActiveRouteRequestGuard {
    control: Arc<RouteExecutionControl>,
}

struct RouteResponseGuard {
    control: Arc<RouteExecutionControl>,
}

#[derive(Clone)]
struct RouteInvocation {
    state: AppState,
    runtime: Arc<RuntimeState>,
    route: IntegrityRoute,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Bytes,
    trailers: GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<String>,
    sampled_execution: bool,
    selected_module: Option<String>,
}

#[derive(Clone, Debug)]
struct RouteServiceError {
    status: StatusCode,
    message: String,
}

impl fmt::Display for RouteServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.status, self.message)
    }
}

impl std::error::Error for RouteServiceError {}

impl From<(StatusCode, String)> for RouteServiceError {
    fn from((status, message): (StatusCode, String)) -> Self {
        Self { status, message }
    }
}

impl From<RouteServiceError> for (StatusCode, String) {
    fn from(error: RouteServiceError) -> Self {
        (error.status, error.message)
    }
}

impl GuestRequest {
    fn new(method: impl Into<String>, uri: impl Into<String>, body: impl Into<Bytes>) -> Self {
        Self {
            method: method.into(),
            uri: uri.into(),
            headers: Vec::new(),
            body: body.into(),
            trailers: Vec::new(),
        }
    }
}

impl GuestHttpResponse {
    fn new(status: StatusCode, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            trailers: Vec::new(),
        }
    }
}

impl GuestResponseBody {
    fn new(
        data: Bytes,
        trailers: Option<HeaderMap>,
        completion_guard: Option<RouteResponseGuard>,
    ) -> Self {
        Self {
            data: Some(data),
            trailers,
            _completion_guard: completion_guard,
        }
    }
}

impl hyper::body::Body for GuestResponseBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
    ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
        if let Some(data) = self.data.take() {
            if !data.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(data))));
            }
        }

        if let Some(trailers) = self.trailers.take() {
            return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
        }

        Poll::Ready(None)
    }

    fn is_end_stream(&self) -> bool {
        self.data.as_ref().map(Bytes::is_empty).unwrap_or(true) && self.trailers.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        let mut hint = SizeHint::default();
        if let Some(data) = &self.data {
            hint.set_exact(data.len() as u64);
        } else {
            hint.set_exact(0);
        }
        hint
    }
}

impl ActiveRouteRequestGuard {
    fn new(control: Arc<RouteExecutionControl>) -> Self {
        control.active_requests.fetch_add(1, Ordering::SeqCst);
        Self { control }
    }

    fn into_response_guard(self) -> RouteResponseGuard {
        let control = Arc::clone(&self.control);
        std::mem::forget(self);
        RouteResponseGuard { control }
    }
}

impl Drop for ActiveRouteRequestGuard {
    fn drop(&mut self) {
        self.control.active_requests.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for RouteResponseGuard {
    fn drop(&mut self) {
        self.control.active_requests.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug)]
struct UdpInboundDatagram {
    source: SocketAddr,
    payload: Bytes,
}

#[cfg(feature = "websockets")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum HostWebSocketFrame {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

#[cfg(feature = "websockets")]
struct HostWebSocketConnection {
    incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum RouteRole {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct HeaderMatch {
    name: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct RetryPolicy {
    #[serde(default)]
    max_retries: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retry_on: Vec<u16>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct ResiliencyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    retry_policy: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct RouteTarget {
    module: String,
    #[serde(default)]
    weight: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    websocket: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    match_header: Option<HeaderMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedRouteTarget {
    module: String,
    websocket: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PropagatedHeader {
    name: String,
    value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct CallerIdentityClaims {
    route_path: String,
    role: RouteRole,
    issued_at: u64,
    expires_at: u64,
}

#[derive(Clone)]
struct HostIdentity {
    signing_key: Arc<SigningKey>,
    public_key: VerifyingKey,
    public_key_hex: String,
}

impl HostIdentity {
    fn generate() -> Self {
        Self::from_signing_key(SigningKey::from_bytes(&rand::random::<[u8; 32]>()))
    }

    fn from_signing_key(signing_key: SigningKey) -> Self {
        let public_key = signing_key.verifying_key();
        Self {
            signing_key: Arc::new(signing_key),
            public_key_hex: hex::encode(public_key.to_bytes()),
            public_key,
        }
    }

    fn sign_route(&self, route: &IntegrityRoute) -> Result<String> {
        let now = unix_timestamp_seconds()?;
        self.sign_claims(&CallerIdentityClaims {
            route_path: normalize_route_path(&route.path),
            role: route.role,
            issued_at: now,
            expires_at: now.saturating_add(IDENTITY_TOKEN_TTL.as_secs()),
        })
    }

    fn sign_claims(&self, claims: &CallerIdentityClaims) -> Result<String> {
        let payload =
            serde_json::to_vec(claims).context("failed to serialize signed caller identity")?;
        let signature = self.signing_key.sign(&payload);
        Ok(format!(
            "{IDENTITY_TOKEN_PREFIX}.{}.{}",
            hex::encode(payload),
            hex::encode(signature.to_bytes())
        ))
    }

    fn verify_header(
        &self,
        headers: &HeaderMap,
    ) -> std::result::Result<CallerIdentityClaims, String> {
        let raw = headers
            .get(TACHYON_IDENTITY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| forbidden_error("missing host-signed caller identity"))?;
        let token = raw
            .strip_prefix("Bearer ")
            .or_else(|| raw.strip_prefix("bearer "))
            .unwrap_or(raw);
        self.verify_token(token)
            .map_err(|error| forbidden_error(&error))
    }

    fn verify_token(&self, token: &str) -> std::result::Result<CallerIdentityClaims, String> {
        let Some(rest) = token.strip_prefix(&format!("{IDENTITY_TOKEN_PREFIX}.")) else {
            return Err("caller identity token has an invalid prefix".to_owned());
        };
        let Some((payload_hex, signature_hex)) = rest.split_once('.') else {
            return Err("caller identity token is malformed".to_owned());
        };
        let payload = hex::decode(payload_hex)
            .map_err(|_| "caller identity token payload is not valid hex".to_owned())?;
        let signature_bytes = decode_hex_array::<64>(signature_hex, "caller identity signature")
            .map_err(|error| error.to_string())?;
        let signature = Signature::from_bytes(&signature_bytes);

        self.public_key
            .verify(&payload, &signature)
            .map_err(|_| "caller identity token signature verification failed".to_owned())?;

        let claims: CallerIdentityClaims = serde_json::from_slice(&payload)
            .map_err(|_| "caller identity token payload is not valid JSON".to_owned())?;
        let now = unix_timestamp_seconds().map_err(|error| error.to_string())?;
        if claims.issued_at > claims.expires_at {
            return Err("caller identity token timestamps are invalid".to_owned());
        }
        if now > claims.expires_at {
            return Err("caller identity token has expired".to_owned());
        }

        Ok(claims)
    }
}

#[cfg(unix)]
impl UdsFastPathRegistry {
    #[cfg(test)]
    fn with_discovery_dir(path: PathBuf) -> Self {
        let registry = Self::default();
        *registry
            .discovery_dir_override
            .lock()
            .expect("UDS discovery override should not be poisoned") = Some(path);
        registry
    }

    fn discovery_dir(&self) -> PathBuf {
        if let Some(path) = self
            .discovery_dir_override
            .lock()
            .expect("UDS discovery override should not be poisoned")
            .clone()
        {
            return path;
        }

        std::env::var_os(TACHYON_DISCOVERY_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DISCOVERY_DIR))
    }

    fn bind_local_listener(&self, config: &IntegrityConfig) -> Result<UnixListener> {
        let discovery_dir = self.discovery_dir();
        fs::create_dir_all(&discovery_dir).with_context(|| {
            format!(
                "failed to create UDS discovery directory `{}`",
                discovery_dir.display()
            )
        })?;

        let host_id = Uuid::new_v4().simple().to_string();
        let file_stem = format!("h-{}", &host_id[..12]);
        let socket_path = discovery_dir.join(format!("{file_stem}.sock"));
        let metadata_path = discovery_dir.join(format!("{file_stem}.json"));
        if socket_path.exists() {
            remove_path_if_exists(&socket_path)?;
        }

        let listener = UnixListener::bind(&socket_path).with_context(|| {
            format!(
                "failed to bind UDS fast-path listener at `{}`",
                socket_path.display()
            )
        })?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o660)).with_context(
            || {
                format!(
                    "failed to tighten permissions on UDS socket `{}`",
                    socket_path.display()
                )
            },
        )?;

        let metadata = UdsPeerMetadata {
            host_id,
            ip: discovery_publish_ip(config)?,
            socket_path: socket_path.display().to_string(),
            protocols: vec!["http/1.1".to_owned(), "h2".to_owned()],
            pressure_state: PeerPressureState::Idle,
            last_pressure_update_unix_ms: 0,
        };
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata)
                .context("failed to serialize UDS peer metadata")?,
        )
        .with_context(|| {
            format!(
                "failed to publish UDS peer metadata `{}`",
                metadata_path.display()
            )
        })?;

        let peer = DiscoveredUdsPeer {
            metadata_path: metadata_path.clone(),
            socket_path: socket_path.clone(),
            metadata: metadata.clone(),
        };
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .insert(metadata.host_id.clone(), peer);
        *self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned") = Some(LocalUdsEndpoint {
            metadata_path,
            socket_path,
        });

        Ok(listener)
    }

    fn discover_peer_for_url(&self, url: &str) -> Option<DiscoveredUdsPeer> {
        let host = reqwest::Url::parse(url).ok()?.host_str()?.to_owned();
        let now_unix_ms = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut candidates = self
            .refresh_peers()
            .into_values()
            .filter(|peer| peer.metadata.ip == host)
            .filter(|peer| {
                peer.metadata.last_pressure_update_unix_ms == 0
                    || now_unix_ms.saturating_sub(peer.metadata.last_pressure_update_unix_ms)
                        <= PEER_PRESSURE_STALE_AFTER.as_millis() as u64
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        if candidates.len() == 1 {
            return candidates.pop();
        }

        let first_index = rand::thread_rng().gen_range(0..candidates.len());
        let mut second_index = rand::thread_rng().gen_range(0..candidates.len() - 1);
        if second_index >= first_index {
            second_index += 1;
        }
        let first = &candidates[first_index];
        let second = &candidates[second_index];
        let preferred = if first.metadata.pressure_state <= second.metadata.pressure_state {
            first
        } else {
            second
        };
        Some(preferred.clone())
    }

    fn active_peer_count(&self) -> usize {
        let peers = self.refresh_peers();
        let local_host = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .as_ref()
            .and_then(|endpoint| {
                fs::read(&endpoint.metadata_path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<UdsPeerMetadata>(&bytes).ok())
                    .map(|metadata| metadata.host_id)
            });

        peers
            .values()
            .filter(|peer| Some(peer.metadata.host_id.clone()) != local_host)
            .count()
    }

    fn write_local_pressure_state(
        &self,
        pressure_state: PeerPressureState,
        updated_at_unix_ms: u64,
    ) -> Result<()> {
        let Some(endpoint) = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .clone()
        else {
            return Ok(());
        };
        let mut metadata: UdsPeerMetadata =
            serde_json::from_slice(&fs::read(&endpoint.metadata_path).with_context(|| {
                format!(
                    "failed to read local UDS metadata `{}`",
                    endpoint.metadata_path.display()
                )
            })?)
            .context("failed to parse local UDS metadata")?;
        metadata.pressure_state = pressure_state;
        metadata.last_pressure_update_unix_ms = updated_at_unix_ms;
        fs::write(
            &endpoint.metadata_path,
            serde_json::to_vec_pretty(&metadata).context("failed to serialize pressure state")?,
        )
        .with_context(|| {
            format!(
                "failed to persist local pressure state to `{}`",
                endpoint.metadata_path.display()
            )
        })?;
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .insert(
                metadata.host_id.clone(),
                DiscoveredUdsPeer {
                    metadata_path: endpoint.metadata_path,
                    socket_path: endpoint.socket_path,
                    metadata,
                },
            );
        Ok(())
    }

    fn note_connect_failure(&self, peer: &DiscoveredUdsPeer) {
        self.peers
            .lock()
            .expect("UDS peer cache should not be poisoned")
            .remove(&peer.metadata.host_id);
        if !peer.socket_path.exists() {
            let _ = fs::remove_file(&peer.metadata_path);
        }
    }

    fn refresh_peers(&self) -> HashMap<String, DiscoveredUdsPeer> {
        let discovery_dir = self.discovery_dir();
        let mut discovered = HashMap::new();
        let entries = match fs::read_dir(&discovery_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.peers
                    .lock()
                    .expect("UDS peer cache should not be poisoned")
                    .clear();
                return discovered;
            }
            Err(_) => {
                return self
                    .peers
                    .lock()
                    .expect("UDS peer cache should not be poisoned")
                    .clone()
            }
        };

        for entry in entries.flatten() {
            let metadata_path = entry.path();
            if metadata_path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let metadata = match fs::read(&metadata_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<UdsPeerMetadata>(&bytes).ok())
            {
                Some(metadata) => metadata,
                None => continue,
            };

            let socket_path = PathBuf::from(&metadata.socket_path);
            if !socket_path.exists() {
                let _ = fs::remove_file(&metadata_path);
                continue;
            }

            discovered.insert(
                metadata.host_id.clone(),
                DiscoveredUdsPeer {
                    metadata_path,
                    socket_path,
                    metadata,
                },
            );
        }

        *self
            .peers
            .lock()
            .expect("UDS peer cache should not be poisoned") = discovered.clone();
        discovered
    }
}

#[cfg(unix)]
impl Drop for UdsFastPathRegistry {
    fn drop(&mut self) {
        if Arc::strong_count(&self.local_endpoint) != 1 {
            return;
        }

        let local_endpoint = self
            .local_endpoint
            .lock()
            .expect("local UDS endpoint should not be poisoned")
            .clone();
        if let Some(endpoint) = local_endpoint {
            let _ = fs::remove_file(endpoint.metadata_path);
            let _ = fs::remove_file(endpoint.socket_path);
        }
    }
}

#[cfg(not(unix))]
impl UdsFastPathRegistry {
    #[cfg(test)]
    #[allow(dead_code)]
    fn with_discovery_dir(_path: PathBuf) -> Self {
        Self
    }

    fn active_peer_count(&self) -> usize {
        0
    }

    fn write_local_pressure_state(
        &self,
        _pressure_state: PeerPressureState,
        _updated_at_unix_ms: u64,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceLimitKind {
    Fuel,
    Memory,
    Stdout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutePermitError {
    Closed,
    TimedOut,
}

#[cfg_attr(not(feature = "secrets-vault"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecretAccessErrorKind {
    NotFound,
    PermissionDenied,
    #[cfg(not(feature = "secrets-vault"))]
    VaultDisabled,
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

struct RouteExecutionControl {
    semaphore: Arc<Semaphore>,
    pending_waiters: AtomicUsize,
    active_requests: AtomicUsize,
    draining: AtomicBool,
    draining_since: Mutex<Option<Instant>>,
    min_instances: u32,
    max_concurrency: u32,
    prewarmed_instances: AtomicUsize,
}

#[derive(Clone)]
struct StorageBrokerManager {
    core_store: Arc<store::CoreStore>,
    queues: Arc<Mutex<HashMap<PathBuf, Arc<StorageVolumeQueue>>>>,
}

struct StorageVolumeQueue {
    volume_root: PathBuf,
    core_store: Arc<store::CoreStore>,
    sender: std::sync::mpsc::Sender<StorageBrokerOperation>,
    state: Mutex<StorageVolumeQueueState>,
    idle: Condvar,
}

#[derive(Default)]
struct StorageVolumeQueueState {
    pending: usize,
}

#[derive(Debug)]
enum StorageBrokerOperation {
    Write(StorageBrokerWriteRequest),
    Snapshot(StorageBrokerSnapshotRequest),
    Restore(StorageBrokerRestoreRequest),
}

#[derive(Clone, Debug)]
struct StorageBrokerWriteRequest {
    route_path: String,
    guest_path: String,
    host_target: PathBuf,
    mode: StorageWriteMode,
    body: Vec<u8>,
}

#[derive(Debug)]
struct StorageBrokerSnapshotRequest {
    volume_id: String,
    source_path: PathBuf,
    snapshot_path: PathBuf,
    completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Debug)]
struct StorageBrokerRestoreRequest {
    volume_id: String,
    snapshot_path: PathBuf,
    destination_path: PathBuf,
    completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StorageWriteMode {
    Overwrite,
    Append,
}

struct ResolvedStorageWriteTarget {
    volume_root: PathBuf,
    guest_path: String,
    host_target: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TtlManagedPath {
    host_path: PathBuf,
    ttl: Duration,
}

#[derive(Clone, Default)]
struct VolumeManager {
    volumes: Arc<Mutex<HashMap<String, Arc<ManagedVolume>>>>,
}

struct ManagedVolume {
    id: String,
    route_path: String,
    guest_path: String,
    active_path: PathBuf,
    snapshot_path: PathBuf,
    idle_timeout: Duration,
    storage_broker: Arc<StorageBrokerManager>,
    state: Mutex<ManagedVolumeState>,
    notify: Notify,
}

struct ManagedVolumeState {
    lifecycle: ManagedVolumeLifecycle,
    active_leases: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManagedVolumeLifecycle {
    Active,
    Hibernating,
    OnDisk,
}

struct ManagedVolumeLease {
    volume: Arc<ManagedVolume>,
}

struct RouteVolumeLeaseGuard {
    leases: Vec<ManagedVolumeLease>,
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

#[derive(Default)]
struct BackgroundWorkerManager {
    workers: Mutex<Vec<BackgroundWorkerHandle>>,
}

struct BackgroundWorkerHandle {
    route_path: String,
    stop_requested: Arc<AtomicBool>,
    join_handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Deserialize)]
struct GuestLogRecord {
    level: String,
    target: Option<String>,
    fields: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum GuestLogStreamType {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct AsyncLogEntry {
    target_name: String,
    timestamp_unix_ms: u64,
    stream_type: GuestLogStreamType,
    level: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    guest_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    structured_fields: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum VolumeType {
    Host,
    Ram,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum VolumeEvictionPolicy {
    Hibernate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
enum RouteQos {
    #[serde(rename = "RealTime", alias = "realtime", alias = "real-time")]
    RealTime,
    #[default]
    #[serde(rename = "Standard", alias = "standard")]
    Standard,
    #[serde(rename = "Batch", alias = "batch")]
    Batch,
}

impl RouteQos {
    #[cfg_attr(not(feature = "ai-inference"), allow(dead_code))]
    fn score(self) -> u16 {
        match self {
            Self::RealTime => 100,
            Self::Standard => 50,
            Self::Batch => 10,
        }
    }
}

fn is_default_route_qos(qos: &RouteQos) -> bool {
    *qos == RouteQos::Standard
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum ModelDevice {
    #[default]
    Cpu,
    Cuda,
    Metal,
    Npu,
    Tpu,
}

impl ModelDevice {
    #[cfg_attr(not(feature = "ai-inference"), allow(dead_code))]
    fn as_str(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Metal => "metal",
            Self::Npu => "npu",
            Self::Tpu => "tpu",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityLayer4Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tcp: Vec<IntegrityTcpBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    udp: Vec<IntegrityUdpBinding>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityTcpBinding {
    port: u16,
    target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityUdpBinding {
    port: u16,
    target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityRoute {
    path: String,
    role: RouteRole,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(
        default = "default_route_version",
        skip_serializing_if = "is_default_route_version"
    )]
    version: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    requires_credentials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    middleware: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<RouteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resiliency: Option<ResiliencyConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<IntegrityModelBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    domains: Vec<String>,
    #[serde(default)]
    min_instances: u32,
    #[serde(default = "default_max_concurrency")]
    max_concurrency: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<IntegrityVolume>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityBatchTarget {
    name: String,
    module: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<IntegrityVolume>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
struct IntegrityModelBinding {
    alias: String,
    path: String,
    #[serde(default, skip_serializing_if = "is_default_model_device")]
    device: ModelDevice,
    #[serde(default, skip_serializing_if = "is_default_route_qos")]
    qos: RouteQos,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityVolume {
    #[serde(
        rename = "type",
        default = "default_volume_type",
        skip_serializing_if = "is_default_volume_type"
    )]
    volume_type: VolumeType,
    host_path: String,
    guest_path: String,
    #[serde(default)]
    readonly: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    idle_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    eviction_policy: Option<VolumeEvictionPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct IntegrityConfig {
    host_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_address: Option<String>,
    max_stdout_bytes: usize,
    guest_fuel_budget: u64,
    guest_memory_limit_bytes: usize,
    resource_limit_response: String,
    #[serde(default, skip_serializing_if = "IntegrityLayer4Config::is_empty")]
    layer4: IntegrityLayer4Config,
    #[serde(
        default = "default_telemetry_sample_rate",
        skip_serializing_if = "is_default_telemetry_sample_rate"
    )]
    telemetry_sample_rate: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    batch_targets: Vec<IntegrityBatchTarget>,
    routes: Vec<IntegrityRoute>,
}

#[derive(Clone, Debug)]
struct ResolvedRoute {
    path: String,
    name: String,
    version: Version,
    dependencies: HashMap<String, VersionReq>,
    requires_credentials: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct RouteRegistry {
    by_name: HashMap<String, Vec<ResolvedRoute>>,
    by_path: HashMap<String, ResolvedRoute>,
}

#[derive(Clone, Debug, Default)]
struct BatchTargetRegistry {
    by_name: HashMap<String, IntegrityBatchTarget>,
}

impl IntegrityLayer4Config {
    fn is_empty(&self) -> bool {
        self.tcp.is_empty() && self.udp.is_empty()
    }
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

#[derive(Debug, Parser)]
#[command(name = "core-host")]
struct HostCli {
    #[command(subcommand)]
    command: Option<HostCommand>,
}

#[derive(Debug, Subcommand)]
enum HostCommand {
    Serve,
    Run(RunCommand),
}

#[derive(Debug, ClapArgs)]
struct RunCommand {
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[arg(long)]
    target: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    init_host_tracing();
    let cli = HostCli::parse();
    match cli.command.unwrap_or(HostCommand::Serve) {
        HostCommand::Serve => serve_host().await,
        HostCommand::Run(command) => {
            let exit_code = match execute_batch_target_from_manifest(
                command.manifest.unwrap_or_else(integrity_manifest_path),
                &command.target,
            )
            .await
            {
                Ok(true) => 0,
                Ok(false) => 1,
                Err(error) => {
                    tracing::error!("batch target `{}` failed: {error:#}", command.target);
                    1
                }
            };
            std::process::exit(exit_code);
        }
    }
}

async fn serve_host() -> Result<()> {
    let manifest_path = integrity_manifest_path();
    let (export_sender, export_receiver) = mpsc::channel(TELEMETRY_EXPORT_QUEUE_CAPACITY);
    let telemetry =
        telemetry::init_telemetry_with_emitter(move |line| export_sender.try_send(line).is_ok());
    let runtime = build_runtime_state(verify_integrity()?)?;
    let core_store = open_core_store_for_manifest(&manifest_path).await?;
    let host_identity = Arc::new(HostIdentity::generate());
    let uds_fast_path = Arc::new(new_uds_fast_path_registry());
    let storage_broker = Arc::new(StorageBrokerManager::new(Arc::clone(&core_store)));
    let buffered_requests = Arc::new(BufferedRequestManager::new(buffered_request_spool_dir(
        &manifest_path,
    )));
    let background_workers = Arc::new(BackgroundWorkerManager::default());
    let tls_manager = Arc::new(tls_runtime::TlsManager::default());
    let (async_log_sender, async_log_receiver) = mpsc::channel(LOG_QUEUE_CAPACITY);
    background_workers.start_for_runtime(
        &runtime,
        telemetry.clone(),
        Arc::clone(&host_identity),
        Arc::clone(&storage_broker),
    );

    let state = AppState {
        runtime: Arc::new(ArcSwap::from_pointee(runtime.clone())),
        draining_runtimes: Arc::new(Mutex::new(Vec::new())),
        http_client: Client::new(),
        async_log_sender,
        secrets_vault: SecretsVault::load(),
        host_identity,
        uds_fast_path: Arc::clone(&uds_fast_path),
        storage_broker,
        core_store,
        buffered_requests,
        volume_manager: Arc::new(VolumeManager::default()),
        telemetry,
        tls_manager,
        manifest_path,
        background_workers: Arc::clone(&background_workers),
    };
    state.tls_manager.prime_from_store(&state).await?;
    prewarm_runtime_routes(
        &runtime,
        state.telemetry.clone(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
    )?;
    spawn_metering_exporter(state.clone(), export_receiver);
    spawn_async_log_exporter(state.clone(), async_log_receiver);
    spawn_reload_watcher(state.clone());
    spawn_draining_runtime_reaper(state.clone());
    spawn_volume_gc_sweeper(state.clone());
    spawn_buffered_request_replayer(state.clone());
    spawn_pressure_monitor(state.clone());
    let app = build_app(state.clone());
    let https_listener = start_https_listener(state.clone(), app.clone()).await?;
    let http3_listener = start_http3_listener(state.clone(), app.clone()).await?;
    let udp_layer4_listeners = start_udp_layer4_listeners(state.clone()).await?;
    let tcp_layer4_listeners = start_tcp_layer4_listeners(state).await?;
    let uds_server = start_uds_fast_path_listener(app.clone(), &runtime.config, uds_fast_path)?;

    let listener = tokio::net::TcpListener::bind(&runtime.config.host_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind HTTP listener on {}",
                runtime.config.host_address.as_str()
            )
        })?;

    tokio::select! {
        result = serve_http_listener(listener, app.clone()) => {
            result.context("HTTP server exited unexpectedly")?;
        }
        _ = shutdown_signal() => {}
    }

    if let Some(server) = uds_server {
        server.abort();
        let _ = server.await;
    }
    if let Some(listener) = https_listener {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    if let Some(listener) = http3_listener {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    for listener in udp_layer4_listeners {
        for handle in listener.join_handles {
            handle.abort();
            let _ = handle.await;
        }
    }
    for listener in tcp_layer4_listeners {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    background_workers.stop_all().await;
    Ok(())
}

async fn execute_batch_target_from_manifest(
    manifest_path: PathBuf,
    target_name: &str,
) -> Result<bool> {
    let config = load_integrity_config_from_manifest_path(&manifest_path)?;
    let target = BatchTargetRegistry::build(&config)?
        .get(target_name)
        .cloned()
        .ok_or_else(|| anyhow!("sealed manifest does not define batch target `{target_name}`"))?;
    let engine = build_command_engine(&config)?;
    let module_path =
        resolve_guest_module_path(&target.module).map_err(|error| anyhow!(error.to_string()))?;
    let component = Component::from_file(&engine, &module_path).map_err(|error| {
        anyhow!(
            "failed to load batch target component `{}` from {}: {error}",
            target.name,
            module_path.display()
        )
    })?;

    let mut linker = ComponentLinker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|error| {
        anyhow!("failed to add WASI preview2 functions to batch target linker: {error}")
    })?;

    let mut wasi = WasiCtxBuilder::new();
    let argv0 = module_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(target.module.as_str())
        .to_owned();
    let args = [argv0.as_str()];
    wasi.inherit_stdio().args(&args);
    for (key, value) in &target.env {
        wasi.env(key, value);
    }
    preopen_batch_target_volumes(&mut wasi, &target)?;

    let mut store = Store::new(
        &engine,
        BatchCommandState {
            ctx: wasi.build(),
            table: ResourceTable::new(),
        },
    );
    let command =
        wasmtime_wasi::p2::bindings::Command::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to instantiate batch target `{}` from {}: {error}",
                    target.name,
                    module_path.display()
                )
            })?;

    let run_result: std::result::Result<(), ()> = command
        .wasi_cli_run()
        .call_run(&mut store)
        .await
        .map_err(|error| anyhow!("failed to execute batch target `{}`: {error}", target.name))?;
    Ok(run_result.is_ok())
}

impl BackgroundWorkerManager {
    fn start_for_runtime(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
    ) {
        let mut new_workers = Vec::new();
        let mut started_workers = 0_u32;

        for route in &runtime.config.routes {
            if route.role != RouteRole::System {
                continue;
            }

            let Some(function_name) = background_route_module(route) else {
                continue;
            };

            if resolve_guest_module_path(&function_name).is_err() {
                tracing::warn!(
                    route = %route.path,
                    function = function_name,
                    "sealed system route is missing its guest artifact"
                );
                continue;
            }

            if BackgroundTickRunner::new(
                &runtime.metered_engine,
                &runtime.config,
                route,
                &function_name,
                telemetry.clone(),
                Arc::clone(&runtime.concurrency_limits),
                Arc::clone(&host_identity),
                Arc::clone(&storage_broker),
            )
            .is_err()
            {
                continue;
            }

            let stop_requested = Arc::new(AtomicBool::new(false));
            let worker_route = route.clone();
            let worker_path = worker_route.path.clone();
            let worker_function_name = function_name.to_owned();
            let worker_engine = runtime.metered_engine.clone();
            let worker_config = runtime.config.clone();
            let worker_telemetry = telemetry.clone();
            let worker_limits = Arc::clone(&runtime.concurrency_limits);
            let worker_host_identity = Arc::clone(&host_identity);
            let worker_storage_broker = Arc::clone(&storage_broker);
            let worker_stop = Arc::clone(&stop_requested);
            let join_handle = tokio::task::spawn_blocking(move || {
                run_background_tick_loop(
                    worker_engine,
                    worker_config,
                    worker_telemetry,
                    worker_limits,
                    worker_host_identity,
                    worker_storage_broker,
                    worker_route,
                    worker_function_name,
                    worker_stop,
                )
            });

            new_workers.push(BackgroundWorkerHandle {
                route_path: worker_path,
                stop_requested,
                join_handle,
            });
            started_workers = started_workers.saturating_add(1);
        }

        if started_workers > 0 {
            tracing::info!(
                workers = started_workers,
                "started autoscaling background workers"
            );
        }

        self.workers
            .lock()
            .expect("background worker list should not be poisoned")
            .extend(new_workers);
    }

    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    async fn replace_with(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
    ) {
        self.stop_all().await;
        self.start_for_runtime(runtime, telemetry, host_identity, storage_broker);
    }

    async fn stop_all(&self) {
        let workers = {
            let mut guard = self
                .workers
                .lock()
                .expect("background worker list should not be poisoned");
            std::mem::take(&mut *guard)
        };

        for worker in &workers {
            worker.stop_requested.store(true, Ordering::Release);
        }

        for worker in workers {
            if let Err(error) = worker.join_handle.await {
                tracing::warn!(
                    route = %worker.route_path,
                    "background worker task exited unexpectedly: {error}"
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_background_tick_loop(
    engine: Engine,
    config: IntegrityConfig,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    route: IntegrityRoute,
    function_name: String,
    stop_requested: Arc<AtomicBool>,
) {
    let mut runner = match BackgroundTickRunner::new(
        &engine,
        &config,
        &route,
        &function_name,
        telemetry,
        concurrency_limits,
        host_identity,
        storage_broker,
    ) {
        Ok(runner) => runner,
        Err(error) => {
            error.log_if_needed(&function_name);
            return;
        }
    };

    loop {
        if !wait_for_background_tick(&stop_requested) {
            break;
        }

        tracing::info!(
            route = %runner.route_path,
            function = %runner.function_name,
            "invoking autoscaling background tick"
        );
        if let Err(error) = runner.tick() {
            error.log_if_needed(&runner.function_name);
        }
    }
}

fn wait_for_background_tick(stop_requested: &AtomicBool) -> bool {
    let deadline = Instant::now() + AUTOSCALING_TICK_INTERVAL;

    loop {
        if stop_requested.load(Ordering::Acquire) {
            return false;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }

        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}

impl StorageBrokerManager {
    fn new(core_store: Arc<store::CoreStore>) -> Self {
        Self {
            core_store,
            queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn enqueue_write_for_route(
        &self,
        route: &IntegrityRoute,
        path: &str,
        mode: StorageWriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let resolved = resolve_storage_write_target(route, path)?;
        self.enqueue_write_target(route.path.clone(), resolved, mode, body)
    }

    fn enqueue_write_target(
        &self,
        route_path: String,
        resolved: ResolvedStorageWriteTarget,
        mode: StorageWriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let queue = self.queue_for_volume(&resolved.volume_root);
        queue.enqueue(StorageBrokerOperation::Write(StorageBrokerWriteRequest {
            route_path,
            guest_path: resolved.guest_path,
            host_target: resolved.host_target,
            mode,
            body,
        }))
    }

    fn enqueue_snapshot(
        &self,
        volume_id: String,
        volume_root: &Path,
        source_path: &Path,
        snapshot_path: &Path,
    ) -> std::result::Result<tokio::sync::oneshot::Receiver<std::result::Result<(), String>>, String>
    {
        let queue = self.queue_for_volume(volume_root);
        let (completion, receiver) = tokio::sync::oneshot::channel();
        queue.enqueue(StorageBrokerOperation::Snapshot(
            StorageBrokerSnapshotRequest {
                volume_id,
                source_path: source_path.to_path_buf(),
                snapshot_path: snapshot_path.to_path_buf(),
                completion,
            },
        ))?;
        Ok(receiver)
    }

    fn enqueue_restore(
        &self,
        volume_id: String,
        volume_root: &Path,
        snapshot_path: &Path,
        destination_path: &Path,
    ) -> std::result::Result<tokio::sync::oneshot::Receiver<std::result::Result<(), String>>, String>
    {
        let queue = self.queue_for_volume(volume_root);
        let (completion, receiver) = tokio::sync::oneshot::channel();
        queue.enqueue(StorageBrokerOperation::Restore(
            StorageBrokerRestoreRequest {
                volume_id,
                snapshot_path: snapshot_path.to_path_buf(),
                destination_path: destination_path.to_path_buf(),
                completion,
            },
        ))?;
        Ok(receiver)
    }

    fn queue_for_volume(&self, volume_root: &Path) -> Arc<StorageVolumeQueue> {
        let key = normalize_path(volume_root.to_path_buf());
        let mut queues = self
            .queues
            .lock()
            .expect("storage broker queues should not be poisoned");
        Arc::clone(
            queues
                .entry(key.clone())
                .or_insert_with(|| StorageVolumeQueue::new(key, Arc::clone(&self.core_store))),
        )
    }

    #[cfg(test)]
    fn wait_for_volume_idle(&self, volume_root: &Path, timeout: Duration) -> bool {
        self.queue_for_volume(volume_root).wait_for_idle(timeout)
    }
}

impl Default for StorageBrokerManager {
    fn default() -> Self {
        let path = std::env::temp_dir().join(format!("tachyon-store-{}.db", Uuid::new_v4()));
        let core_store =
            store::CoreStore::open(&path).expect("default storage broker core store should open");
        Self::new(Arc::new(core_store))
    }
}

impl StorageVolumeQueue {
    fn new(volume_root: PathBuf, core_store: Arc<store::CoreStore>) -> Arc<Self> {
        let (sender, receiver) = std::sync::mpsc::channel::<StorageBrokerOperation>();
        let queue = Arc::new(Self {
            volume_root,
            core_store,
            sender,
            state: Mutex::new(StorageVolumeQueueState::default()),
            idle: Condvar::new(),
        });
        let worker = Arc::clone(&queue);
        std::thread::spawn(move || worker.run(receiver));
        queue
    }

    fn enqueue(&self, operation: StorageBrokerOperation) -> std::result::Result<(), String> {
        self.state
            .lock()
            .expect("storage broker queue state should not be poisoned")
            .pending += 1;
        if self.sender.send(operation).is_ok() {
            return Ok(());
        }

        let mut state = self
            .state
            .lock()
            .expect("storage broker queue state should not be poisoned");
        state.pending = state.pending.saturating_sub(1);
        self.idle.notify_all();
        Err(format!(
            "storage broker queue for `{}` is not available",
            self.volume_root.display()
        ))
    }

    fn run(self: Arc<Self>, receiver: std::sync::mpsc::Receiver<StorageBrokerOperation>) {
        while let Ok(operation) = receiver.recv() {
            match operation {
                StorageBrokerOperation::Write(request) => {
                    if let Err(error) = process_storage_write_request(&request) {
                        tracing::warn!(
                            route = %request.route_path,
                            guest_path = %request.guest_path,
                            host_target = %request.host_target.display(),
                            "storage broker write failed: {error}"
                        );
                    }
                }
                StorageBrokerOperation::Snapshot(request) => {
                    let result = process_storage_snapshot_request(&request, &self.core_store)
                        .map_err(|error| format!("{error:#}"));
                    if let Err(error) = &result {
                        tracing::warn!(
                            volume_id = %request.volume_id,
                            snapshot_path = %request.snapshot_path.display(),
                            "storage broker snapshot failed: {error}"
                        );
                    }
                    let _ = request.completion.send(result);
                }
                StorageBrokerOperation::Restore(request) => {
                    let result = process_storage_restore_request(&request, &self.core_store)
                        .map_err(|error| format!("{error:#}"));
                    if let Err(error) = &result {
                        tracing::warn!(
                            volume_id = %request.volume_id,
                            snapshot_path = %request.snapshot_path.display(),
                            destination_path = %request.destination_path.display(),
                            "storage broker restore failed: {error}"
                        );
                    }
                    let _ = request.completion.send(result);
                }
            }

            let mut state = self
                .state
                .lock()
                .expect("storage broker queue state should not be poisoned");
            state.pending = state.pending.saturating_sub(1);
            self.idle.notify_all();
        }
    }

    #[cfg(test)]
    fn wait_for_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .expect("storage broker queue state should not be poisoned");

        while state.pending > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }

            let (next_state, result) = self
                .idle
                .wait_timeout(state, remaining)
                .expect("storage broker queue state should not be poisoned");
            state = next_state;
            if result.timed_out() && state.pending > 0 {
                return false;
            }
        }

        true
    }
}

impl BufferedRequestManager {
    fn new(disk_dir: PathBuf) -> Self {
        fs::create_dir_all(&disk_dir).unwrap_or_else(|error| {
            panic!(
                "buffered request spool directory `{}` should initialize: {error}",
                disk_dir.display()
            )
        });
        Self {
            disk_dir,
            ram_capacity: BUFFER_RAM_REQUEST_CAPACITY,
            total_capacity: BUFFER_TOTAL_REQUEST_CAPACITY,
            state: Arc::new(Mutex::new(BufferedRequestState {
                next_id: 0,
                ram_queue: VecDeque::new(),
                disk_queue: VecDeque::new(),
            })),
            notify: Arc::new(Notify::new()),
        }
    }

    fn enqueue(
        &self,
        request: BufferedRouteRequest,
    ) -> std::result::Result<(oneshot::Receiver<BufferedRouteResult>, BufferedRequestTier), String>
    {
        let (completion, receiver) = oneshot::channel();
        let mut state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        let total_queued = state.ram_queue.len() + state.disk_queue.len();
        if total_queued >= self.total_capacity {
            return Err("buffer manager is full".to_owned());
        }

        let id = format!("buffered-{}", state.next_id);
        state.next_id = state.next_id.saturating_add(1);
        if state.ram_queue.len() < self.ram_capacity {
            state.ram_queue.push_back(BufferedMemoryRequest {
                id,
                request,
                completion,
            });
            drop(state);
            self.notify.notify_one();
            Ok((receiver, BufferedRequestTier::Ram))
        } else {
            let path = self.disk_dir.join(format!("{id}.json"));
            let payload = serde_json::to_vec(&request)
                .map_err(|error| format!("failed to serialize buffered request: {error}"))?;
            fs::write(&path, payload).map_err(|error| {
                format!(
                    "failed to persist buffered request spool file `{}`: {error}",
                    path.display()
                )
            })?;
            state.disk_queue.push_back(BufferedDiskRequest {
                id,
                path,
                completion,
            });
            drop(state);
            self.notify.notify_one();
            Ok((receiver, BufferedRequestTier::Disk))
        }
    }

    fn pop_next(&self) -> std::result::Result<Option<BufferedQueueItem>, String> {
        let queued = {
            let mut state = self
                .state
                .lock()
                .expect("buffered request state should not be poisoned");
            if let Some(request) = state.ram_queue.pop_front() {
                return Ok(Some(BufferedQueueItem {
                    id: request.id,
                    request: request.request,
                    completion: request.completion,
                    disk_path: None,
                }));
            }
            state.disk_queue.pop_front()
        };

        let Some(request) = queued else {
            return Ok(None);
        };
        let payload = fs::read(&request.path).map_err(|error| {
            format!(
                "failed to read buffered request spool file `{}`: {error}",
                request.path.display()
            )
        })?;
        let buffered = serde_json::from_slice(&payload)
            .map_err(|error| format!("failed to deserialize buffered request: {error}"))?;
        Ok(Some(BufferedQueueItem {
            id: request.id,
            request: buffered,
            completion: request.completion,
            disk_path: Some(request.path),
        }))
    }

    fn requeue_front(&self, item: BufferedQueueItem) -> std::result::Result<(), String> {
        let mut state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        match item.disk_path {
            Some(path) => state.disk_queue.push_front(BufferedDiskRequest {
                id: item.id,
                path,
                completion: item.completion,
            }),
            None => state.ram_queue.push_front(BufferedMemoryRequest {
                id: item.id,
                request: item.request,
                completion: item.completion,
            }),
        }
        drop(state);
        self.notify.notify_one();
        Ok(())
    }

    fn complete(&self, item: BufferedQueueItem, result: BufferedRouteResult) {
        if let Some(path) = &item.disk_path {
            let _ = fs::remove_file(path);
        }
        let _ = item.completion.send(result);
    }

    fn pending_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        state.ram_queue.len() + state.disk_queue.len()
    }

    #[cfg(test)]
    fn disk_spill_count(&self) -> usize {
        self.state
            .lock()
            .expect("buffered request state should not be poisoned")
            .disk_queue
            .len()
    }
}

impl VolumeManager {
    async fn acquire_route_volumes(
        &self,
        route: &IntegrityRoute,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<RouteVolumeLeaseGuard, String> {
        let mut leases = Vec::new();
        for volume in route
            .volumes
            .iter()
            .filter(|volume| volume.is_hibernation_capable())
        {
            let managed = self.managed_volume(route, volume, Arc::clone(&storage_broker))?;
            leases.push(managed.acquire().await?);
        }

        Ok(RouteVolumeLeaseGuard { leases })
    }

    fn managed_volume(
        &self,
        route: &IntegrityRoute,
        volume: &IntegrityVolume,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<Arc<ManagedVolume>, String> {
        let key = managed_volume_key(&route.path, &volume.guest_path);
        let mut volumes = self
            .volumes
            .lock()
            .expect("managed volume registry should not be poisoned");
        if let Some(volume) = volumes.get(&key) {
            return Ok(Arc::clone(volume));
        }

        let managed = Arc::new(ManagedVolume::new(&route.path, volume, storage_broker)?);
        volumes.insert(key, Arc::clone(&managed));
        Ok(managed)
    }

    #[cfg(test)]
    fn managed_volume_for_route(
        &self,
        route_path: &str,
        guest_path: &str,
    ) -> Option<Arc<ManagedVolume>> {
        self.volumes
            .lock()
            .expect("managed volume registry should not be poisoned")
            .get(&managed_volume_key(route_path, guest_path))
            .cloned()
    }
}

impl ManagedVolume {
    fn new(
        route_path: &str,
        volume: &IntegrityVolume,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<Self, String> {
        let active_path = normalize_path(PathBuf::from(&volume.host_path));
        fs::create_dir_all(&active_path).map_err(|error| {
            format!(
                "failed to initialize RAM volume directory `{}` for route `{route_path}`: {error}",
                active_path.display()
            )
        })?;

        Ok(Self {
            id: managed_volume_id(route_path, &volume.guest_path),
            route_path: route_path.to_owned(),
            guest_path: volume.guest_path.clone(),
            snapshot_path: snapshot_path_for_volume(&active_path),
            active_path,
            idle_timeout: volume
                .parsed_idle_timeout()
                .map_err(|error| format!("{error:#}"))?
                .ok_or_else(|| {
                    format!(
                        "route `{route_path}` volume `{}` is missing an `idle_timeout` for hibernation",
                        volume.guest_path
                    )
                })?,
            state: Mutex::new(ManagedVolumeState {
                lifecycle: ManagedVolumeLifecycle::Active,
                active_leases: 0,
                generation: 0,
            }),
            notify: Notify::new(),
            storage_broker,
        })
    }

    async fn acquire(self: &Arc<Self>) -> std::result::Result<ManagedVolumeLease, String> {
        loop {
            let should_restore = {
                let mut state = self
                    .state
                    .lock()
                    .expect("managed volume state should not be poisoned");
                match state.lifecycle {
                    ManagedVolumeLifecycle::Active => {
                        state.active_leases = state.active_leases.saturating_add(1);
                        state.generation = state.generation.saturating_add(1);
                        return Ok(ManagedVolumeLease {
                            volume: Arc::clone(self),
                        });
                    }
                    ManagedVolumeLifecycle::OnDisk => {
                        state.lifecycle = ManagedVolumeLifecycle::Hibernating;
                        state.generation = state.generation.saturating_add(1);
                        true
                    }
                    ManagedVolumeLifecycle::Hibernating => false,
                }
            };

            if should_restore {
                let completion = self.storage_broker.enqueue_restore(
                    self.id.clone(),
                    &self.active_path,
                    &self.snapshot_path,
                    &self.active_path,
                )?;
                match completion.await {
                    Ok(Ok(())) => self.finish_restore(ManagedVolumeLifecycle::Active),
                    Ok(Err(error)) => {
                        self.finish_restore(ManagedVolumeLifecycle::OnDisk);
                        return Err(format!(
                            "failed to restore hibernated volume `{}`: {error}",
                            self.id
                        ));
                    }
                    Err(_) => {
                        self.finish_restore(ManagedVolumeLifecycle::OnDisk);
                        return Err(format!(
                            "storage broker restore completion channel closed for volume `{}`",
                            self.id
                        ));
                    }
                }
                continue;
            }

            self.notify.notified().await;
        }
    }

    fn release(self: &Arc<Self>) {
        let generation = {
            let mut state = self
                .state
                .lock()
                .expect("managed volume state should not be poisoned");
            state.active_leases = state.active_leases.saturating_sub(1);
            state.generation = state.generation.saturating_add(1);
            if state.lifecycle == ManagedVolumeLifecycle::Active && state.active_leases == 0 {
                Some(state.generation)
            } else {
                None
            }
        };

        if let Some(generation) = generation {
            self.schedule_hibernation(generation);
        }
        self.notify.notify_waiters();
    }

    fn schedule_hibernation(self: &Arc<Self>, generation: u64) {
        let volume = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(volume.idle_timeout).await;

            let should_snapshot = {
                let mut state = volume
                    .state
                    .lock()
                    .expect("managed volume state should not be poisoned");
                if state.lifecycle != ManagedVolumeLifecycle::Active
                    || state.active_leases != 0
                    || state.generation != generation
                {
                    return;
                }

                state.lifecycle = ManagedVolumeLifecycle::Hibernating;
                state.generation = state.generation.saturating_add(1);
                true
            };

            if !should_snapshot {
                return;
            }

            let completion = match volume.storage_broker.enqueue_snapshot(
                volume.id.clone(),
                &volume.active_path,
                &volume.active_path,
                &volume.snapshot_path,
            ) {
                Ok(completion) => completion,
                Err(error) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "failed to schedule hibernation snapshot: {error}"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                    return;
                }
            };

            match completion.await {
                Ok(Ok(())) => volume.finish_restore(ManagedVolumeLifecycle::OnDisk),
                Ok(Err(error)) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "hibernation snapshot failed: {error}"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                }
                Err(_) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "hibernation snapshot completion channel closed unexpectedly"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                }
            }
        });
    }

    fn finish_restore(&self, lifecycle: ManagedVolumeLifecycle) {
        let mut state = self
            .state
            .lock()
            .expect("managed volume state should not be poisoned");
        state.lifecycle = lifecycle;
        state.generation = state.generation.saturating_add(1);
        self.notify.notify_waiters();
    }

    #[cfg(test)]
    fn lifecycle(&self) -> ManagedVolumeLifecycle {
        self.state
            .lock()
            .expect("managed volume state should not be poisoned")
            .lifecycle
    }
}

impl Drop for ManagedVolumeLease {
    fn drop(&mut self) {
        self.volume.release();
    }
}

impl Drop for RouteVolumeLeaseGuard {
    fn drop(&mut self) {
        let _ = self.leases.len();
    }
}

async fn run_volume_gc_tick(runtime: Arc<RuntimeState>) -> Result<()> {
    let managed_paths = collect_ttl_managed_paths(&runtime.config);
    let mut handles = Vec::with_capacity(managed_paths.len());

    for managed_path in managed_paths {
        handles.push(tokio::task::spawn_blocking(move || {
            sweep_ttl_managed_path(&managed_path)
        }));
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!("volume GC worker failed: {error:#}"),
            Err(error) => tracing::warn!("volume GC blocking task failed: {error}"),
        }
    }

    Ok(())
}

fn collect_ttl_managed_paths(config: &IntegrityConfig) -> Vec<TtlManagedPath> {
    let mut deduped = BTreeMap::<PathBuf, Duration>::new();

    for route in &config.routes {
        for volume in &route.volumes {
            let Some(ttl_seconds) = volume.ttl_seconds else {
                continue;
            };
            let ttl = Duration::from_secs(ttl_seconds);
            let host_path = normalize_path(PathBuf::from(&volume.host_path));
            deduped
                .entry(host_path)
                .and_modify(|existing| {
                    if ttl < *existing {
                        *existing = ttl;
                    }
                })
                .or_insert(ttl);
        }
    }

    deduped
        .into_iter()
        .map(|(host_path, ttl)| TtlManagedPath { host_path, ttl })
        .collect()
}

fn sweep_ttl_managed_path(managed_path: &TtlManagedPath) -> Result<()> {
    let read_dir = match fs::read_dir(&managed_path.host_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read TTL-managed path `{}`",
                    managed_path.host_path.display()
                )
            })
        }
    };

    for entry in read_dir {
        let entry = entry.with_context(|| {
            format!(
                "failed to enumerate an entry inside TTL-managed path `{}`",
                managed_path.host_path.display()
            )
        })?;
        let entry_path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read metadata for TTL-managed entry `{}`",
                        entry_path.display()
                    )
                })
            }
        };
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read modified time for TTL-managed entry `{}`",
                        entry_path.display()
                    )
                })
            }
        };

        if !ttl_entry_is_stale(modified, managed_path.ttl) {
            continue;
        }

        if let Err(error) = remove_stale_ttl_entry(&entry_path, metadata.is_dir()) {
            tracing::warn!(
                path = %entry_path.display(),
                "volume GC failed to remove stale entry gracefully: {error:#}"
            );
        }
    }

    Ok(())
}

fn ttl_entry_is_stale(modified: SystemTime, ttl: Duration) -> bool {
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= ttl)
}

fn remove_stale_ttl_entry(path: &Path, is_dir: bool) -> Result<()> {
    let result = if is_dir {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };

    match result {
        Ok(()) => {
            tracing::info!(path = %path.display(), "volume GC removed stale entry");
            Ok(())
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove stale TTL-managed entry `{}`",
                path.display()
            )
        }),
    }
}

fn resolve_storage_write_target(
    route: &IntegrityRoute,
    path: &str,
) -> std::result::Result<ResolvedStorageWriteTarget, String> {
    let normalized_path =
        normalize_guest_volume_path(path).map_err(|error| format!("{error:#}"))?;
    let volume = route
        .volumes
        .iter()
        .filter(|volume| guest_path_matches_volume(&normalized_path, &volume.guest_path))
        .max_by_key(|volume| volume.guest_path.len())
        .ok_or_else(|| {
            format!(
                "route `{}` cannot broker writes to `{normalized_path}` because no mounted volume matches that path",
                route.path
            )
        })?;

    let relative_path = normalized_path
        .strip_prefix(&volume.guest_path)
        .unwrap_or_default()
        .trim_start_matches('/');
    if relative_path.is_empty() {
        return Err(format!(
            "storage broker path `{normalized_path}` must target a file beneath mounted guest path `{}`",
            volume.guest_path
        ));
    }

    let volume_root = normalize_path(PathBuf::from(&volume.host_path));
    let mut host_target = volume_root.clone();
    for segment in relative_path.split('/') {
        host_target.push(segment);
    }

    Ok(ResolvedStorageWriteTarget {
        volume_root,
        guest_path: normalized_path,
        host_target,
    })
}

fn parse_storage_broker_host_path(
    value: &str,
    label: &str,
) -> std::result::Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("storage broker `{label}` must not be empty"));
    }

    Ok(PathBuf::from(trimmed))
}

fn authorize_storage_broker_write(
    config: &IntegrityConfig,
    headers: &HeaderMap,
    host_identity: &HostIdentity,
    path: &str,
) -> std::result::Result<(IntegrityRoute, ResolvedStorageWriteTarget), String> {
    let claims = host_identity.verify_header(headers)?;
    let route = config
        .sealed_route(&claims.route_path)
        .cloned()
        .ok_or_else(|| {
            forbidden_error(&format!(
                "signed caller route `{}` is not sealed in `integrity.lock`",
                claims.route_path
            ))
        })?;
    if route.role != claims.role {
        return Err(forbidden_error(&format!(
            "signed caller role mismatch for route `{}`",
            claims.route_path
        )));
    }

    let resolved =
        resolve_storage_write_target(&route, path).map_err(|error| forbidden_error(&error))?;
    Ok((route, resolved))
}

fn guest_path_matches_volume(path: &str, guest_path: &str) -> bool {
    path == guest_path
        || path
            .strip_prefix(guest_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn process_storage_write_request(request: &StorageBrokerWriteRequest) -> Result<()> {
    if let Some(parent) = request.host_target.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create broker parent directory for {}",
                request.host_target.display()
            )
        })?;
    }

    match request.mode {
        StorageWriteMode::Overwrite => {
            fs::write(&request.host_target, &request.body).with_context(|| {
                format!(
                    "failed to overwrite {} through storage broker",
                    request.host_target.display()
                )
            })
        }
        StorageWriteMode::Append => {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&request.host_target)
                .with_context(|| {
                    format!(
                        "failed to open {} for append through storage broker",
                        request.host_target.display()
                    )
                })?;
            file.write_all(&request.body).with_context(|| {
                format!(
                    "failed to append to {} through storage broker",
                    request.host_target.display()
                )
            })
        }
    }
}

fn process_storage_snapshot_request(
    request: &StorageBrokerSnapshotRequest,
    core_store: &store::CoreStore,
) -> Result<()> {
    let _ = &request.snapshot_path;
    core_store
        .snapshot_directory(&request.volume_id, &request.source_path)
        .with_context(|| {
            format!(
                "failed to persist hibernation snapshot for volume `{}`",
                request.volume_id
            )
        })?;
    remove_path_if_exists(&request.source_path)?;
    Ok(())
}

fn process_storage_restore_request(
    request: &StorageBrokerRestoreRequest,
    core_store: &store::CoreStore,
) -> Result<()> {
    let restored = core_store
        .restore_directory(&request.volume_id, &request.destination_path)
        .with_context(|| {
            format!(
                "failed to restore hibernation snapshot for volume `{}`",
                request.volume_id
            )
        })?;
    if restored {
        return Ok(());
    }

    copy_directory_tree(&request.snapshot_path, &request.destination_path)
}

fn copy_directory_tree(source: &Path, destination: &Path) -> Result<()> {
    remove_path_if_exists(destination)?;
    fs::create_dir_all(destination).with_context(|| {
        format!(
            "failed to create destination directory `{}`",
            destination.display()
        )
    })?;

    if !source.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read directory `{}`", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry inside `{}`", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry.metadata().with_context(|| {
            format!(
                "failed to read metadata for broker copy source `{}`",
                source_path.display()
            )
        })?;

        if metadata.is_dir() {
            copy_directory_tree(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create destination parent directory `{}`",
                        parent.display()
                    )
                })?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy `{}` to `{}`",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for `{}`", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory `{}`", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove file `{}`", path.display()))?;
    }

    Ok(())
}

fn managed_volume_key(route_path: &str, guest_path: &str) -> String {
    format!("{route_path}:{guest_path}")
}

fn managed_volume_id(route_path: &str, guest_path: &str) -> String {
    format!(
        "{}:{}",
        route_path.trim_matches('/').replace('/', "_"),
        guest_path.trim_matches('/').replace('/', "_")
    )
}

fn snapshot_path_for_volume(active_path: &Path) -> PathBuf {
    let mut snapshot = active_path.to_path_buf();
    snapshot.set_extension("snapshot");
    snapshot
}

#[cfg(unix)]
fn spawn_reload_watcher(state: AppState) {
    tokio::spawn(async move {
        let mut hangup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        {
            Ok(signal) => signal,
            Err(error) => {
                tracing::warn!("failed to install SIGHUP watcher: {error}");
                return;
            }
        };

        while hangup.recv().await.is_some() {
            if let Err(error) = reload_runtime_from_disk(&state).await {
                tracing::error!(
                    manifest = %state.manifest_path.display(),
                    "hot reload failed: {error:#}"
                );
            }
        }
    });
}

#[cfg(not(unix))]
fn spawn_reload_watcher(_state: AppState) {}

fn spawn_volume_gc_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(VOLUME_GC_TICK_INTERVAL);

        loop {
            interval.tick().await;
            let runtime = state.runtime.load_full();
            if let Err(error) = run_volume_gc_tick(runtime).await {
                tracing::warn!("volume GC sweep failed: {error:#}");
            }
        }
    });
}

fn spawn_buffered_request_replayer(state: AppState) {
    tokio::spawn(async move {
        loop {
            state.buffered_requests.notify.notified().await;

            loop {
                if state.buffered_requests.pending_count() == 0 {
                    break;
                }

                let runtime = state.runtime.load_full();
                if telemetry::active_requests(&state.telemetry)
                    >= PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                {
                    tokio::time::sleep(BUFFER_REPLAY_RETRY_INTERVAL).await;
                    continue;
                }

                let Some(buffered) = state.buffered_requests.pop_next().unwrap_or_else(|error| {
                    tracing::warn!("failed to load buffered request: {error}");
                    None
                }) else {
                    break;
                };

                let Some(route) = runtime
                    .config
                    .sealed_route(&buffered.request.route_path)
                    .cloned()
                else {
                    state.buffered_requests.complete(
                        buffered,
                        Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            "buffered route is no longer sealed".to_owned(),
                        )),
                    );
                    continue;
                };
                let Some(semaphore) = runtime.concurrency_limits.get(&route.path).cloned() else {
                    state.buffered_requests.complete(
                        buffered,
                        Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "buffered route is missing a concurrency limiter".to_owned(),
                        )),
                    );
                    continue;
                };

                let permit = match Arc::clone(&semaphore.semaphore).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(TryAcquireError::NoPermits) => {
                        let _ = state.buffered_requests.requeue_front(buffered);
                        tokio::time::sleep(BUFFER_REPLAY_RETRY_INTERVAL).await;
                        continue;
                    }
                    Err(TryAcquireError::Closed) => {
                        state.buffered_requests.complete(
                            buffered,
                            Err((
                                StatusCode::SERVICE_UNAVAILABLE,
                                format!("route `{}` is currently unavailable", route.path),
                            )),
                        );
                        continue;
                    }
                };

                let result = execute_buffered_route_request(
                    &state,
                    &runtime,
                    &route,
                    semaphore,
                    permit,
                    buffered.request.clone(),
                )
                .await;
                state.buffered_requests.complete(buffered, result);
            }
        }
    });
}

fn spawn_pressure_monitor(state: AppState) {
    tokio::spawn(async move {
        let mut previous_state = PeerPressureState::Idle;
        loop {
            let peer_count = state.uds_fast_path.active_peer_count();
            if peer_count == 0 {
                tokio::time::sleep(PRESSURE_MONITOR_IDLE_SLEEP_INTERVAL).await;
                continue;
            }

            let runtime = state.runtime.load_full();
            let active_requests = telemetry::active_requests(&state.telemetry);
            let pending_requests = runtime
                .concurrency_limits
                .values()
                .map(|control| control.pending_queue_size() as usize)
                .sum::<usize>();
            let saturated_entry = active_requests >= PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                || pending_requests >= PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD;
            let saturated_exit = active_requests
                < PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                    .saturating_sub(PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD)
                && pending_requests == 0;
            let caution_entry = active_requests >= PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD
                || pending_requests > 0;
            let caution_exit = active_requests
                < (PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD / 2).max(1)
                && pending_requests == 0;
            let pressure_state = match previous_state {
                PeerPressureState::Saturated if !saturated_exit => PeerPressureState::Saturated,
                PeerPressureState::Caution if !caution_exit && !saturated_entry => {
                    PeerPressureState::Caution
                }
                _ if saturated_entry => PeerPressureState::Saturated,
                _ if caution_entry => PeerPressureState::Caution,
                _ => PeerPressureState::Idle,
            };
            let now_unix_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or_default();
            if let Err(error) = state
                .uds_fast_path
                .write_local_pressure_state(pressure_state, now_unix_ms)
            {
                tracing::debug!("failed to update local pressure metadata: {error:#}");
            }
            previous_state = pressure_state;
            tokio::time::sleep(PRESSURE_MONITOR_POLL_INTERVAL).await;
        }
    });
}

fn spawn_draining_runtime_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DRAINING_REAPER_TICK_INTERVAL);

        loop {
            interval.tick().await;
            run_draining_runtime_reaper_tick(&state);
        }
    });
}

fn run_draining_runtime_reaper_tick(state: &AppState) {
    let now = Instant::now();
    let mut draining_runtimes = state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned");
    let mut retained = Vec::with_capacity(draining_runtimes.len());

    for draining in draining_runtimes.drain(..) {
        let active_requests = draining.runtime.active_request_count();
        let timed_out =
            now.saturating_duration_since(draining.draining_since) >= DRAINING_ROUTE_TIMEOUT;
        if active_requests == 0 || timed_out {
            if timed_out && active_requests > 0 {
                for control in draining.runtime.concurrency_limits.values() {
                    control.force_terminate();
                }
            }

            tracing::info!(
                active_requests,
                forced = timed_out && active_requests > 0,
                drained_routes = draining.runtime.draining_route_count(),
                "graceful draining reaped an inactive runtime generation"
            );
            continue;
        }

        retained.push(draining);
    }

    *draining_runtimes = retained;
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
async fn reload_runtime_from_disk(state: &AppState) -> Result<()> {
    let manifest_path = state.manifest_path.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        let config = load_integrity_config_from_manifest_path(&manifest_path)?;
        build_runtime_state(config)
    })
    .await
    .context("hot reload task failed")??;
    prewarm_runtime_routes(
        &runtime,
        state.telemetry.clone(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
    )?;
    let previous_runtime = state.runtime.load_full();
    let draining_since = Instant::now();
    previous_runtime.mark_draining(draining_since);
    state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned")
        .push(DrainingRuntime {
            runtime: previous_runtime,
            draining_since,
        });

    state
        .background_workers
        .replace_with(
            &runtime,
            state.telemetry.clone(),
            Arc::clone(&state.host_identity),
            Arc::clone(&state.storage_broker),
        )
        .await;
    let runtime = Arc::new(runtime);
    state.runtime.store(Arc::clone(&runtime));
    run_draining_runtime_reaper_tick(state);
    tracing::info!(
        manifest = %state.manifest_path.display(),
        draining_generations = state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        "Hot reload successful"
    );
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let ctrl_c = tokio::signal::ctrl_c();
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!("failed to install SIGTERM watcher: {error}");
                let _ = ctrl_c.await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    tracing::info!("shutdown signal received");
}

fn build_runtime_state(config: IntegrityConfig) -> Result<RuntimeState> {
    Ok(RuntimeState {
        engine: build_engine(&config, false)?,
        metered_engine: build_engine(&config, true)?,
        route_registry: Arc::new(RouteRegistry::build(&config)?),
        batch_target_registry: Arc::new(BatchTargetRegistry::build(&config)?),
        concurrency_limits: build_concurrency_limits(&config),
        #[cfg(feature = "ai-inference")]
        ai_runtime: Arc::new(ai_inference::AiInferenceRuntime::from_config(&config)?),
        config,
    })
}

impl RuntimeState {
    fn mark_draining(&self, started_at: Instant) {
        for control in self.concurrency_limits.values() {
            control.mark_draining(started_at);
        }
    }

    fn active_request_count(&self) -> usize {
        self.concurrency_limits
            .values()
            .map(|control| control.active_request_count())
            .sum()
    }

    fn draining_route_count(&self) -> usize {
        self.concurrency_limits
            .values()
            .filter(|control| control.lifecycle_state() == RouteLifecycleState::Draining)
            .count()
    }
}

fn prewarm_runtime_routes(
    runtime: &RuntimeState,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> Result<()> {
    let mut warmed_routes = 0_u32;
    let mut warmed_instances = 0_u32;

    for route in &runtime.config.routes {
        let Some(control) = runtime.concurrency_limits.get(&route.path) else {
            continue;
        };
        if control.min_instances == 0 {
            continue;
        }

        let modules = route_modules_for_prewarm(route);
        for function_name in modules {
            for _ in 0..control.min_instances {
                prewarm_route_instance(
                    runtime,
                    route,
                    &function_name,
                    telemetry.clone(),
                    Arc::clone(&host_identity),
                    Arc::clone(&storage_broker),
                )?;
                control.record_prewarm_success();
                warmed_instances = warmed_instances.saturating_add(1);
            }
        }

        tracing::info!(
            route = %route.path,
            min_instances = control.min_instances,
            max_concurrency = control.max_concurrency,
            "prewarmed route capacity"
        );
        warmed_routes = warmed_routes.saturating_add(1);
    }

    if warmed_instances > 0 {
        tracing::info!(
            routes = warmed_routes,
            instances = warmed_instances,
            "completed instance pool prewarming"
        );
    }

    Ok(())
}

fn route_modules_for_prewarm(route: &IntegrityRoute) -> Vec<String> {
    let mut modules = BTreeSet::new();

    if route.targets.is_empty() {
        modules.insert(default_route_name(&route.path));
    } else {
        for target in &route.targets {
            modules.insert(target.module.clone());
        }
    }

    modules.into_iter().collect()
}

#[cfg(feature = "ai-inference")]
fn add_accelerator_interfaces_to_component_linker(
    linker: &mut ComponentLinker<ComponentHostState>,
    ai_runtime: &ai_inference::AiInferenceRuntime,
    context: &str,
) -> std::result::Result<(), ExecutionError> {
    accelerator_component_bindings::tachyon::accelerator::cpu::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            format!("failed to add CPU accelerator functions to {context}"),
        )
    })?;

    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Gpu) {
        accelerator_component_bindings::tachyon::accelerator::gpu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add GPU accelerator functions to {context}"),
            )
        })?;
    }
    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Npu) {
        accelerator_component_bindings::tachyon::accelerator::npu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add NPU accelerator functions to {context}"),
            )
        })?;
    }
    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Tpu) {
        accelerator_component_bindings::tachyon::accelerator::tpu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add TPU accelerator functions to {context}"),
            )
        })?;
    }

    Ok(())
}

fn prewarm_route_instance(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> Result<()> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;

    if let Ok(component) = Component::from_file(&runtime.engine, &module_path) {
        match prewarm_component_route(
            runtime,
            route,
            function_name,
            &module_path,
            &component,
            telemetry.clone(),
            Arc::clone(&host_identity),
            Arc::clone(&storage_broker),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if should_fall_back_from_component_prewarm(&error) => {}
            Err(error) => {
                return Err(anyhow!(
                    "failed to prewarm component `{}` for route `{}`: {}",
                    module_path.display(),
                    route.path,
                    execution_error_text(&error)
                ));
            }
        }
    }

    prewarm_legacy_route(runtime, route, function_name, &module_path, host_identity).map_err(
        |error| {
            anyhow!(
                "failed to prewarm guest `{function_name}`: {}",
                execution_error_text(&error)
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn prewarm_component_route(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> std::result::Result<(), ExecutionError> {
    if route.role == RouteRole::System {
        match BackgroundTickRunner::new(
            &runtime.engine,
            &runtime.config,
            route,
            function_name,
            telemetry.clone(),
            Arc::clone(&runtime.concurrency_limits),
            Arc::clone(&host_identity),
            Arc::clone(&storage_broker),
        ) {
            Ok(_) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }

        match prewarm_system_component_instance(
            &runtime.engine,
            runtime.config.clone(),
            runtime.config.guest_memory_limit_bytes,
            route,
            module_path,
            component,
            telemetry,
            host_identity,
            storage_broker,
            Arc::clone(&runtime.concurrency_limits),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }
    } else {
        if route_has_udp_binding(&runtime.config, function_name) {
            match prewarm_udp_component_instance(
                &runtime.engine,
                runtime.config.clone(),
                runtime.config.guest_memory_limit_bytes,
                route,
                module_path,
                component,
                telemetry.clone(),
                host_identity.clone(),
                storage_broker.clone(),
                Arc::clone(&runtime.concurrency_limits),
            ) {
                Ok(()) => return Ok(()),
                Err(error) if !should_fall_back_from_component_prewarm(&error) => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }

        #[cfg(feature = "websockets")]
        if route
            .targets
            .iter()
            .any(|target| target.websocket && target.module == function_name)
        {
            match prewarm_websocket_component_instance(
                &runtime.engine,
                runtime.config.clone(),
                runtime.config.guest_memory_limit_bytes,
                route,
                module_path,
                component,
                telemetry.clone(),
                host_identity.clone(),
                storage_broker.clone(),
                Arc::clone(&runtime.concurrency_limits),
            ) {
                Ok(()) => return Ok(()),
                Err(error) if !should_fall_back_from_component_prewarm(&error) => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }

        match prewarm_http_component_instance(
            &runtime.engine,
            runtime.config.clone(),
            runtime.config.guest_memory_limit_bytes,
            route,
            module_path,
            component,
            telemetry,
            host_identity,
            storage_broker,
            Arc::clone(&runtime.concurrency_limits),
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }
    }

    Err(ExecutionError::Internal(format!(
        "component `{}` did not match a supported prewarm world",
        module_path.display()
    )))
}

#[allow(clippy::too_many_arguments)]
fn prewarm_http_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm HTTP component linker",
        )
    })?;
    #[cfg(feature = "ai-inference")]
    add_accelerator_interfaces_to_component_linker(
        &mut linker,
        ai_runtime.as_ref(),
        "prewarm HTTP component linker",
    )?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    #[cfg(feature = "ai-inference")]
    {
        store.data_mut().ai_runtime = Some(ai_runtime);
    }
    store.limiter(|state| &mut state.limits);
    let _ = component_bindings::FaasGuest::instantiate(&mut store, component, &linker).map_err(
        |error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        },
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prewarm_udp_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm UDP component linker",
        )
    })?;
    udp_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm UDP component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    let _ = udp_component_bindings::UdpFaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate UDP guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        })?;
    Ok(())
}

#[cfg(feature = "websockets")]
#[allow(clippy::too_many_arguments)]
fn prewarm_websocket_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::websocket::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WebSocket functions to prewarm component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    let _ = websocket_component_bindings::WebsocketFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        guest_execution_error(
            error,
            format!(
                "failed to instantiate WebSocket guest component during prewarm from {}",
                module_path.display()
            ),
        )
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn prewarm_system_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::storage_broker::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add storage broker functions to prewarm system component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    let _ = system_component_bindings::SystemFaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate system guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        })?;
    Ok(())
}

fn prewarm_legacy_route(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    module_path: &Path,
    host_identity: Arc<HostIdentity>,
) -> std::result::Result<(), ExecutionError> {
    let module = Module::from_file(&runtime.engine, module_path).map_err(|error| {
        guest_execution_error(
            error,
            format!(
                "failed to load legacy guest artifact during prewarm from {}",
                module_path.display()
            ),
        )
    })?;
    let linker = build_linker(&runtime.engine)?;
    let stdin_file = create_guest_stdin_file(&Bytes::new())?;
    let stdout_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stdout,
        disconnected_log_sender(),
        false,
        runtime.config.max_stdout_bytes,
    );
    let stderr_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stderr,
        disconnected_log_sender(),
        false,
        0,
    );

    let mut wasi = WasiCtxBuilder::new();
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(InputFile::new(stdin_file.file.try_clone().map_err(
            |error| {
                guest_execution_error(
                    error.into(),
                    "failed to clone prewarm guest stdin file handle",
                )
            },
        )?))
        .stdout(stdout_capture)
        .stderr(stderr_capture);
    add_route_environment(&mut wasi, route, host_identity.as_ref())?;

    if let Some(module_dir) = module_path.parent() {
        wasi.preopened_dir(module_dir, ".", DirPerms::READ, FilePerms::READ)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to preopen guest module directory {} during prewarm",
                        module_dir.display()
                    ),
                )
            })?;
    }

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        &runtime.engine,
        LegacyHostState::new(
            wasi,
            runtime.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    let _instance = linker.instantiate(&mut store, &module).map_err(|error| {
        guest_execution_error(error, "failed to instantiate legacy guest during prewarm")
    })?;
    Ok(())
}

fn route_has_udp_binding(config: &IntegrityConfig, function_name: &str) -> bool {
    let normalized = normalize_target_module_name(function_name);
    config
        .layer4
        .udp
        .iter()
        .any(|binding| normalize_target_module_name(&binding.target) == normalized)
}

fn should_fall_back_from_component_prewarm(error: &ExecutionError) -> bool {
    match error {
        ExecutionError::Internal(message) => {
            message.contains("no exported instance named")
                || message.contains("does not export")
                || message.contains("on-connect")
                || message.contains("handle-packet")
        }
        _ => false,
    }
}

fn execution_error_text(error: &ExecutionError) -> String {
    match error {
        ExecutionError::GuestModuleNotFound(details) => details.to_string(),
        ExecutionError::ResourceLimitExceeded { detail, .. } => detail.clone(),
        ExecutionError::Internal(message) => message.clone(),
    }
}

fn integrity_manifest_path() -> PathBuf {
    std::env::var_os(INTEGRITY_MANIFEST_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("integrity.lock"))
}

fn build_app(state: AppState) -> Router {
    let app = Router::new()
        .fallback(faas_handler)
        .layer(from_fn(hop_limit_middleware));

    let app = app.layer(axum::middleware::from_fn_with_state(
        state.clone(),
        custom_domain_routing_middleware,
    ));

    #[cfg(feature = "rate-limit")]
    let app = app.layer(axum::middleware::from_fn_with_state(
        rate_limit::new_rate_limiter(),
        rate_limit::rate_limit_middleware,
    ));

    app.with_state(state)
}

fn should_sample_telemetry(sample_rate: f64) -> bool {
    sample_rate > 0.0 && rand::thread_rng().gen_bool(sample_rate.clamp(0.0, 1.0))
}

fn merge_fuel_samples(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn generate_traceparent() -> String {
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = format!("{:016x}", rand::thread_rng().gen::<u64>());
    format!("00-{trace_id}-{span_id}-01")
}

fn encode_metering_batch(batch: Vec<String>) -> Bytes {
    let mut payload = batch.join("\n");
    if !payload.is_empty() {
        payload.push('\n');
    }
    Bytes::from(payload)
}

async fn export_metering_batch(
    state: &AppState,
    batch: Vec<String>,
) -> std::result::Result<(), String> {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.sealed_route(SYSTEM_METERING_ROUTE).cloned() else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_METERING_ROUTE);
    let body = encode_metering_batch(batch);
    let trailers = Vec::new();
    let result = execute_route_with_middleware(
        state,
        &runtime,
        &route,
        &headers,
        &method,
        &uri,
        &body,
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        None,
        false,
        None,
    )
    .await
    .map_err(|(status, message)| format!("metering route failed with {status}: {message}"))?;

    if result.response.status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "metering route returned HTTP {}",
            result.response.status
        ))
    }
}

fn spawn_metering_exporter(state: AppState, mut receiver: mpsc::Receiver<String>) {
    tokio::spawn(async move {
        while let Some(first_record) = receiver.recv().await {
            let mut batch = vec![first_record];
            while batch.len() < TELEMETRY_EXPORT_BATCH_SIZE {
                match receiver.try_recv() {
                    Ok(record) => batch.push(record),
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
            }

            if let Err(error) = export_metering_batch(&state, batch).await {
                tracing::warn!("telemetry metering export failed: {error}");
            }
        }
    });
}

fn spawn_async_log_exporter(state: AppState, mut receiver: mpsc::Receiver<AsyncLogEntry>) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async move {
        while let Some(first_entry) = receiver.recv().await {
            let mut batch = vec![first_entry];
            while batch.len() < LOG_BATCH_SIZE {
                match tokio::time::timeout(LOG_BATCH_FLUSH_INTERVAL, receiver.recv()).await {
                    Ok(Some(entry)) => batch.push(entry),
                    Ok(None) | Err(_) => break,
                }
            }

            if let Err(error) = export_log_batch(&state, batch).await {
                tracing::warn!("async guest log export failed: {error}");
            }
        }
    });
}

async fn serve_http_listener(listener: tokio::net::TcpListener, app: Router) -> Result<()> {
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .context("failed to accept HTTP connection")?;
        let service = app.clone();
        tokio::spawn(async move {
            let builder = HyperConnectionBuilder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(
                TokioIo::new(stream),
                TowerToHyperService::new(service),
            );
            if let Err(error) = connection.await {
                tracing::warn!(remote = %peer_addr, "HTTP connection failed: {error}");
            }
        });
    }
}

async fn export_log_batch(
    state: &AppState,
    batch: Vec<AsyncLogEntry>,
) -> std::result::Result<(), String> {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.sealed_route(SYSTEM_LOGGER_ROUTE).cloned() else {
        return Ok(());
    };

    let headers = HeaderMap::new();
    let method = Method::POST;
    let uri = Uri::from_static(SYSTEM_LOGGER_ROUTE);
    let body = serde_json::to_vec(&batch)
        .map_err(|error| format!("failed to serialize log batch: {error}"))?;
    let trailers = Vec::new();
    let result = execute_route_with_middleware(
        state,
        &runtime,
        &route,
        &headers,
        &method,
        &uri,
        &Bytes::from(body),
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        None,
        false,
        None,
    )
    .await
    .map_err(|(status, message)| format!("logger route failed with {status}: {message}"))?;

    if result.response.status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "logger route returned unexpected status {}",
            result.response.status
        ))
    }
}

#[cfg(unix)]
fn start_uds_fast_path_listener(
    app: Router,
    config: &IntegrityConfig,
    registry: Arc<UdsFastPathRegistry>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let listener = registry.bind_local_listener(config)?;
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!("UDS fast-path listener accept failed: {error}");
                    break;
                }
            };

            let service = app.clone();
            tokio::spawn(async move {
                let builder = HyperConnectionBuilder::new(TokioExecutor::new());
                let connection = builder.serve_connection_with_upgrades(
                    TokioIo::new(stream),
                    TowerToHyperService::new(service),
                );
                if let Err(error) = connection.await {
                    tracing::warn!("UDS fast-path connection failed: {error}");
                }
            });
        }
    });

    Ok(Some(handle))
}

#[cfg(not(unix))]
fn start_uds_fast_path_listener(
    _app: Router,
    _config: &IntegrityConfig,
    _registry: Arc<UdsFastPathRegistry>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    Ok(None)
}

fn layer4_bind_address(host_address: &str, port: u16) -> Result<SocketAddr> {
    let mut address = host_address.parse::<SocketAddr>().with_context(|| {
        format!("failed to parse `host_address` `{host_address}` for Layer 4 binding")
    })?;
    address.set_port(port);
    Ok(address)
}

fn https_bind_address(config: &IntegrityConfig) -> Result<Option<SocketAddr>> {
    if !config.has_custom_domains() {
        return Ok(None);
    }

    if let Some(address) = &config.tls_address {
        return address
            .parse()
            .with_context(|| format!("invalid tls_address `{address}`"))
            .map(Some);
    }

    let mut address = config.host_address.parse::<SocketAddr>().with_context(|| {
        format!(
            "failed to parse `host_address` `{}` for HTTPS binding",
            config.host_address
        )
    })?;
    address.set_port(443);
    Ok(Some(address))
}

async fn start_https_listener(state: AppState, app: Router) -> Result<Option<HttpsListenerHandle>> {
    let runtime = state.runtime.load_full();
    let Some(bind_address) = https_bind_address(&runtime.config)? else {
        return Ok(None);
    };

    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .with_context(|| format!("failed to bind HTTPS listener on {bind_address}"))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read HTTPS listener local address")?;

    let join_handle = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!("HTTPS listener accept failed: {error}");
                    continue;
                }
            };
            let connection_state = state.clone();
            let connection_app = app.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_https_connection(connection_state, connection_app, stream).await
                {
                    tracing::warn!(remote = %peer_addr, "HTTPS connection failed: {error:#}");
                }
            });
        }
    });

    Ok(Some(HttpsListenerHandle {
        local_addr,
        join_handle,
    }))
}

async fn handle_https_connection(
    state: AppState,
    app: Router,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let start = LazyConfigAcceptor::new(tokio_rustls::rustls::server::Acceptor::default(), stream)
        .await
        .context("failed to accept TLS client hello")?;
    let client_hello = start.client_hello();
    let domain = client_hello
        .server_name()
        .ok_or_else(|| anyhow!("TLS client hello did not include SNI"))?;
    let config = state
        .tls_manager
        .server_config_for_domain(&state, domain)
        .await?;
    let tls_stream = start
        .into_stream(config)
        .await
        .context("failed to complete rustls handshake")?;

    HyperConnectionBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(tls_stream), TowerToHyperService::new(app))
        .await
        .map_err(|error| anyhow!("HTTPS connection exited unexpectedly: {error}"))
}

#[cfg(feature = "http3")]
async fn start_http3_listener(state: AppState, app: Router) -> Result<Option<Http3ListenerHandle>> {
    server_h3::start_http3_listener(state, app).await
}

#[cfg(not(feature = "http3"))]
async fn start_http3_listener(
    _state: AppState,
    _app: Router,
) -> Result<Option<Http3ListenerHandle>> {
    Ok(None)
}

async fn start_udp_layer4_listeners(state: AppState) -> Result<Vec<UdpLayer4ListenerHandle>> {
    start_udp_layer4_listeners_with_queue_capacity(state, UDP_LAYER4_QUEUE_CAPACITY).await
}

async fn start_udp_layer4_listeners_with_queue_capacity(
    state: AppState,
    queue_capacity: usize,
) -> Result<Vec<UdpLayer4ListenerHandle>> {
    let runtime = state.runtime.load_full();
    let mut listeners = Vec::new();

    for binding in &runtime.config.layer4.udp {
        let resolved = runtime
            .route_registry
            .resolve_named_route(&binding.target)
            .map_err(|error| {
                anyhow!(
                    "invalid UDP Layer 4 binding target `{}`: {error}",
                    binding.target
                )
            })?;
        let route = runtime
            .config
            .sealed_route(&resolved.path)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "UDP Layer 4 binding target `{}` resolved to a missing route",
                    binding.target
                )
            })?;
        let bind_address = layer4_bind_address(&runtime.config.host_address, binding.port)?;
        let socket = Arc::new(
            tokio::net::UdpSocket::bind(bind_address)
                .await
                .with_context(|| {
                    format!("failed to bind UDP Layer 4 listener on {bind_address}")
                })?,
        );
        let local_addr = socket
            .local_addr()
            .context("failed to read bound UDP Layer 4 listener address")?;
        let (tx, rx) = mpsc::channel::<UdpInboundDatagram>(queue_capacity.max(1));
        let rx = Arc::new(TokioMutex::new(rx));
        let listener_socket = Arc::clone(&socket);
        let listener_target = binding.target.clone();
        let listener_handle = tokio::spawn(async move {
            let mut buffer = vec![0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
            loop {
                let (size, source) = match listener_socket.recv_from(&mut buffer).await {
                    Ok(received) => received,
                    Err(error) => {
                        tracing::warn!(
                            port = local_addr.port(),
                            target = listener_target,
                            "UDP Layer 4 listener receive failed: {error}"
                        );
                        break;
                    }
                };

                let packet = UdpInboundDatagram {
                    source,
                    payload: Bytes::copy_from_slice(&buffer[..size]),
                };
                if let Err(error) = tx.try_send(packet) {
                    match error {
                        mpsc::error::TrySendError::Full(_) => {
                            tracing::warn!(
                                port = local_addr.port(),
                                remote = %source,
                                target = listener_target,
                                "dropping UDP datagram because the safe queue threshold was exceeded"
                            );
                        }
                        mpsc::error::TrySendError::Closed(_) => break,
                    }
                }
            }
        });

        let mut join_handles = vec![listener_handle];
        for _ in 0..udp_listener_worker_count(route.max_concurrency) {
            let worker_state = state.clone();
            let worker_route = route.clone();
            let worker_socket = Arc::clone(&socket);
            let worker_rx = Arc::clone(&rx);
            let worker_target = binding.target.clone();
            join_handles.push(tokio::spawn(async move {
                loop {
                    let packet = {
                        let mut receiver = worker_rx.lock().await;
                        receiver.recv().await
                    };
                    let Some(packet) = packet else {
                        break;
                    };
                    if let Err(error) = handle_udp_layer4_datagram(
                        worker_state.clone(),
                        worker_route.clone(),
                        Arc::clone(&worker_socket),
                        packet,
                    )
                    .await
                    {
                        tracing::warn!(
                            target = %worker_target,
                            "UDP Layer 4 datagram failed: {error:#}"
                        );
                    }
                }
            }));
        }

        listeners.push(UdpLayer4ListenerHandle {
            local_addr,
            join_handles,
        });
    }

    Ok(listeners)
}

fn udp_listener_worker_count(max_concurrency: u32) -> usize {
    usize::try_from(max_concurrency)
        .ok()
        .map(|count| count.clamp(1, UDP_LAYER4_MAX_WORKERS_PER_LISTENER))
        .unwrap_or(UDP_LAYER4_MAX_WORKERS_PER_LISTENER)
}

async fn start_tcp_layer4_listeners(state: AppState) -> Result<Vec<TcpLayer4ListenerHandle>> {
    let runtime = state.runtime.load_full();
    let mut listeners = Vec::new();

    for binding in &runtime.config.layer4.tcp {
        let resolved = runtime
            .route_registry
            .resolve_named_route(&binding.target)
            .map_err(|error| {
                anyhow!(
                    "invalid TCP Layer 4 binding target `{}`: {error}",
                    binding.target
                )
            })?;
        let route = runtime
            .config
            .sealed_route(&resolved.path)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "TCP Layer 4 binding target `{}` resolved to a missing route",
                    binding.target
                )
            })?;
        let bind_address = layer4_bind_address(&runtime.config.host_address, binding.port)?;
        let listener = tokio::net::TcpListener::bind(bind_address)
            .await
            .with_context(|| format!("failed to bind TCP Layer 4 listener on {bind_address}"))?;
        let local_addr = listener
            .local_addr()
            .context("failed to read bound TCP Layer 4 listener address")?;
        let listener_state = state.clone();
        let listener_route = route.clone();
        let listener_target = binding.target.clone();
        let join_handle = tokio::spawn(async move {
            loop {
                let (stream, remote_addr) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!(
                            port = local_addr.port(),
                            target = listener_target,
                            "TCP Layer 4 listener accept failed: {error}"
                        );
                        break;
                    }
                };

                let connection_state = listener_state.clone();
                let connection_route = listener_route.clone();
                let connection_target = connection_route.name.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_tcp_layer4_connection(connection_state, connection_route, stream)
                            .await
                    {
                        tracing::warn!(
                            target = %connection_target,
                            remote = %remote_addr,
                            "TCP Layer 4 connection failed: {error:#}"
                        );
                    }
                });
            }
        });

        listeners.push(TcpLayer4ListenerHandle {
            local_addr,
            join_handle,
        });
    }

    Ok(listeners)
}

async fn handle_udp_layer4_datagram(
    state: AppState,
    route: IntegrityRoute,
    socket: Arc<tokio::net::UdpSocket>,
    datagram: UdpInboundDatagram,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire UDP Layer 4 volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "UDP Layer 4 route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = match acquire_route_permit(semaphore).await {
        Ok(permit) => permit,
        Err(RoutePermitError::Closed) => return Ok(()),
        Err(RoutePermitError::TimedOut) => {
            tracing::warn!(
                route = %route.path,
                remote = %datagram.source,
                "dropping UDP datagram because the route is saturated"
            );
            return Ok(());
        }
    };
    let function_name = select_stream_route_module(&route)
        .map_err(|error| anyhow!("failed to resolve UDP Layer 4 target module: {error}"))?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let runtime_telemetry = state.telemetry.clone();
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let request_headers = HeaderMap::new();
    let route_for_execution = route.clone();
    let source = datagram.source;
    let payload = datagram.payload;
    let responses = tokio::task::spawn_blocking(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let execution = GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry,
            async_log_sender: state.async_log_sender.clone(),
            secret_access: SecretAccess::from_route(&route_for_execution, &SecretsVault::load()),
            request_headers,
            host_identity,
            storage_broker,
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
        };
        execute_udp_layer4_guest(
            &engine,
            &route_for_execution,
            &function_name,
            source,
            payload,
            &execution,
        )
    })
    .await
    .context("UDP Layer 4 worker exited before returning a result")?
    .map_err(|error| anyhow!("UDP Layer 4 guest failed: {error:?}"))?;

    for response in responses {
        socket
            .send_to(&response.payload, response.target)
            .await
            .with_context(|| format!("failed to send UDP datagram to {}", response.target))?;
    }

    Ok(())
}

#[cfg(feature = "websockets")]
async fn handle_websocket_connection(
    state: AppState,
    route: IntegrityRoute,
    function_name: String,
    socket: WebSocket,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire WebSocket route volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "WebSocket route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = acquire_route_permit(semaphore)
        .await
        .map_err(|error| match error {
            RoutePermitError::Closed => anyhow!("WebSocket route `{}` is unavailable", route.path),
            RoutePermitError::TimedOut => anyhow!("WebSocket route `{}` is saturated", route.path),
        })?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let runtime_telemetry = state.telemetry.clone();
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let secret_access = SecretAccess::from_route(&route, &state.secrets_vault);
    let (incoming_tx, incoming_rx) = std::sync::mpsc::channel::<HostWebSocketFrame>();
    let (outgoing_tx, mut outgoing_rx) =
        tokio::sync::mpsc::unbounded_channel::<HostWebSocketFrame>();
    let (mut writer, mut reader) = socket.split();

    let reader_handle = tokio::spawn(async move {
        while let Some(message) = reader.next().await {
            match message {
                Ok(message) => {
                    let frame = websocket_message_to_host_frame(message);
                    let should_close = matches!(frame, HostWebSocketFrame::Close);
                    if incoming_tx.send(frame).is_err() || should_close {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!("WebSocket receive failed: {error}");
                    let _ = incoming_tx.send(HostWebSocketFrame::Close);
                    break;
                }
            }
        }
    });

    let writer_handle = tokio::spawn(async move {
        while let Some(frame) = outgoing_rx.recv().await {
            let should_close = matches!(frame, HostWebSocketFrame::Close);
            if writer
                .send(host_frame_to_websocket_message(frame))
                .await
                .is_err()
            {
                break;
            }
            if should_close {
                break;
            }
        }
        let _ = writer.close().await;
    });

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let execution = GuestExecutionContext {
            config,
            sampled_execution: false,
            runtime_telemetry,
            async_log_sender: state.async_log_sender.clone(),
            secret_access,
            request_headers: HeaderMap::new(),
            host_identity,
            storage_broker,
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
        };
        let _ = result_tx.send(execute_websocket_guest(
            &engine,
            &route,
            &function_name,
            incoming_rx,
            outgoing_tx,
            &execution,
        ));
    });

    let result = result_rx
        .await
        .context("WebSocket guest thread exited before returning a result")?;
    let _ = reader_handle.await;
    let _ = writer_handle.await;
    result.map_err(|error| anyhow!("WebSocket guest failed: {error:?}"))?;
    Ok(())
}

async fn handle_tcp_layer4_connection(
    state: AppState,
    route: IntegrityRoute,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire TCP Layer 4 volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "TCP Layer 4 route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = acquire_route_permit(semaphore)
        .await
        .map_err(|error| match error {
            RoutePermitError::Closed => {
                anyhow!("TCP Layer 4 route `{}` is unavailable", route.path)
            }
            RoutePermitError::TimedOut => {
                anyhow!("TCP Layer 4 route `{}` is saturated", route.path)
            }
        })?;
    let function_name = select_stream_route_module(&route)
        .map_err(|error| anyhow!("failed to resolve TCP Layer 4 target module: {error}"))?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    if !route.domains.is_empty() {
        return handle_tls_wrapped_tcp_layer4_connection(
            state,
            route,
            stream,
            function_name,
            engine,
            config,
            volume_leases,
            permit,
            runtime,
        )
        .await;
    }

    let socket = stream
        .into_std()
        .context("failed to convert TCP Layer 4 socket into std mode")?;
    socket
        .set_nonblocking(false)
        .context("failed to set TCP Layer 4 socket into blocking mode")?;
    let stdin_socket = socket
        .try_clone()
        .context("failed to clone TCP Layer 4 socket for guest stdin")?;
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let telemetry = state.telemetry.clone();
    #[cfg(feature = "ai-inference")]
    let ai_runtime = Arc::clone(&runtime.ai_runtime);

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let _ = result_tx.send(execute_tcp_layer4_guest(
            &engine,
            &config,
            &route,
            &function_name,
            TcpSocketStdin::new(stdin_socket),
            TcpSocketStdout::new(socket),
            telemetry,
            host_identity,
            storage_broker,
            concurrency_limits,
            #[cfg(feature = "ai-inference")]
            ai_runtime,
        ));
    });
    result_rx
        .await
        .context("TCP Layer 4 guest thread exited before returning a result")??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_tls_wrapped_tcp_layer4_connection(
    state: AppState,
    route: IntegrityRoute,
    stream: tokio::net::TcpStream,
    function_name: String,
    engine: Engine,
    config: IntegrityConfig,
    volume_leases: RouteVolumeLeaseGuard,
    permit: OwnedSemaphorePermit,
    runtime: Arc<RuntimeState>,
) -> Result<()> {
    let start = LazyConfigAcceptor::new(tokio_rustls::rustls::server::Acceptor::default(), stream)
        .await
        .context("failed to accept TLS client hello for Layer 4 route")?;
    let client_hello = start.client_hello();
    let domain = tls_runtime::normalize_domain(
        client_hello
            .server_name()
            .ok_or_else(|| anyhow!("TLS Layer 4 client hello did not include SNI"))?,
    )?;
    if !route.domains.iter().any(|candidate| candidate == &domain) {
        return Err(anyhow!(
            "TLS Layer 4 route `{}` does not allow SNI `{domain}`",
            route.path
        ));
    }

    let tls_config = state
        .tls_manager
        .server_config_for_domain(&state, &domain)
        .await?;
    let mut tls_stream = start
        .into_stream(tls_config)
        .await
        .context("failed to complete TLS handshake for Layer 4 route")?;

    let bridge_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("failed to bind local TLS bridge listener")?;
    let bridge_addr = bridge_listener
        .local_addr()
        .context("failed to resolve TLS bridge listener address")?;
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let telemetry = state.telemetry.clone();
    #[cfg(feature = "ai-inference")]
    let ai_runtime = Arc::clone(&runtime.ai_runtime);

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let result = (|| -> std::result::Result<(), ExecutionError> {
            let (socket, _) = bridge_listener.accept().map_err(|error| {
                guest_execution_error(error.into(), "failed to accept TLS bridge socket")
            })?;
            let stdin_socket = socket.try_clone().map_err(|error| {
                guest_execution_error(error.into(), "failed to clone TLS bridge socket")
            })?;
            execute_tcp_layer4_guest(
                &engine,
                &config,
                &route,
                &function_name,
                TcpSocketStdin::new(stdin_socket),
                TcpSocketStdout::new(socket),
                telemetry,
                host_identity,
                storage_broker,
                concurrency_limits,
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            )
        })();
        let _ = result_tx.send(result);
    });

    let mut bridge_stream = tokio::net::TcpStream::connect(bridge_addr)
        .await
        .context("failed to connect local TLS bridge stream")?;
    tokio::io::copy_bidirectional(&mut tls_stream, &mut bridge_stream)
        .await
        .context("failed to proxy decrypted TLS Layer 4 stream")?;

    result_rx
        .await
        .context("TLS Layer 4 guest thread exited before returning a result")??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_tcp_layer4_guest(
    engine: &Engine,
    config: &IntegrityConfig,
    route: &IntegrityRoute,
    function_name: &str,
    stdin_stream: TcpSocketStdin,
    stdout_stream: TcpSocketStdout,
    runtime_telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
) -> std::result::Result<(), ExecutionError> {
    let execution = GuestExecutionContext {
        config: config.clone(),
        sampled_execution: false,
        runtime_telemetry,
        async_log_sender: disconnected_log_sender(),
        secret_access: SecretAccess::from_route(route, &SecretsVault::load()),
        request_headers: HeaderMap::new(),
        host_identity,
        storage_broker,
        telemetry: None,
        concurrency_limits,
        propagated_headers: Vec::new(),
        #[cfg(feature = "ai-inference")]
        ai_runtime,
    };
    let (module_path, module) = resolve_legacy_guest_module(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        "default",
    )?;
    execute_legacy_guest_with_stdio(
        engine,
        route,
        &module_path,
        module,
        &execution,
        stdin_stream,
        stdout_stream,
    )
}

async fn hop_limit_middleware(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let hop_limit = match resolve_incoming_hop_limit(req.headers()) {
        Ok(hop_limit) => hop_limit,
        Err(()) => return loop_detected_response(),
    };

    req.extensions_mut().insert(hop_limit);
    req.headers_mut()
        .insert(HOP_LIMIT_HEADER, hop_limit.as_header_value());

    next.run(req).await
}

async fn custom_domain_routing_middleware(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(host) = request_host(req.headers()) else {
        return next.run(req).await;
    };
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.route_for_domain(host) else {
        return next.run(req).await;
    };
    let path = route_domain_request_path(route, req.uri());
    let mut builder = Uri::builder();
    if let Some(scheme) = req.uri().scheme_str() {
        builder = builder.scheme(scheme);
    }
    if let Some(authority) = req.uri().authority().cloned() {
        builder = builder.authority(authority);
    }
    if let Ok(uri) = builder.path_and_query(path).build() {
        *req.uri_mut() = uri;
    }

    next.run(req).await
}

fn route_domain_request_path(route: &IntegrityRoute, uri: &Uri) -> String {
    let original_path = normalize_route_path(uri.path());
    let path = if original_path == "/" {
        route.path.clone()
    } else {
        format!("{}{}", route.path, original_path)
    };

    match uri.query() {
        Some(query) => format!("{path}?{query}"),
        None => path,
    }
}

fn request_host(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(':').next().unwrap_or(value))
}

fn header_map_to_guest_fields(headers: &HeaderMap) -> GuestHttpFields {
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

fn guest_fields_to_header_map(
    fields: &GuestHttpFields,
    label: &str,
) -> std::result::Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    insert_guest_fields(&mut headers, fields, label)?;
    Ok(headers)
}

fn insert_guest_fields(
    target: &mut HeaderMap,
    fields: &GuestHttpFields,
    label: &str,
) -> std::result::Result<(), String> {
    for (name, value) in fields {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| format!("guest returned an invalid {label} name `{name}`: {error}"))?;
        let header_value = HeaderValue::from_str(value).map_err(|error| {
            format!("guest returned an invalid {label} value for `{name}`: {error}")
        })?;
        target.append(header_name, header_value);
    }

    Ok(())
}

fn build_guest_response(
    response: GuestHttpResponse,
    completion_guard: Option<RouteResponseGuard>,
) -> std::result::Result<Response, String> {
    let mut response_headers = HeaderMap::new();
    insert_guest_fields(&mut response_headers, &response.headers, "response header")?;

    let trailer_map = if response.trailers.is_empty() {
        None
    } else {
        let mut trailers = HeaderMap::new();
        insert_guest_fields(&mut trailers, &response.trailers, "response trailer")?;
        Some(trailers)
    };

    let mut built = Response::builder()
        .status(response.status)
        .body(Body::new(GuestResponseBody::new(
            response.body,
            trailer_map,
            completion_guard,
        )))
        .map_err(|error| format!("failed to construct guest HTTP response: {error}"))?;
    built.headers_mut().extend(response_headers);
    Ok(built)
}

fn guest_response_into_response(result: RouteExecutionResult) -> Response {
    match build_guest_response(result.response, result.completion_guard) {
        Ok(response) => response,
        Err(message) => (StatusCode::INTERNAL_SERVER_ERROR, message).into_response(),
    }
}

#[cfg(not(feature = "resiliency"))]
mod resiliency {
    use super::{execute_route_with_middleware_inner, RouteExecutionResult, RouteInvocation};
    use axum::http::StatusCode;

    pub(crate) async fn execute_route_with_resiliency(
        invocation: RouteInvocation,
    ) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
        execute_route_with_middleware_inner(&invocation).await
    }
}

async fn faas_handler(
    State(state): State<AppState>,
    Extension(hop_limit): Extension<HopLimit>,
    #[cfg(feature = "websockets")] ws: Option<WebSocketUpgrade>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let headers = parts.headers;
    let method = parts.method;
    let uri = parts.uri;
    let collected = match body.collect().await {
        Ok(collected) => collected,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to read request body: {error}"),
            )
                .into_response();
        }
    };
    let trailers = collected.trailers().cloned().unwrap_or_default();
    let body = collected.to_bytes();
    let trailer_fields = header_map_to_guest_fields(&trailers);
    let _active_request = telemetry::begin_request(&state.telemetry);
    let runtime = state.runtime.load_full();
    let normalized_path = normalize_route_path(uri.path());
    let trace_id = Uuid::new_v4().to_string();
    let sampled_execution = normalized_path != SYSTEM_METERING_ROUTE
        && should_sample_telemetry(runtime.config.telemetry_sample_rate);
    let traceparent = sampled_execution.then(generate_traceparent);
    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestStart {
            trace_id: trace_id.clone(),
            path: normalized_path.clone(),
            sampled: sampled_execution,
            traceparent: traceparent.clone(),
            timestamp: Instant::now(),
        },
    );

    let (response, fuel_consumed): (Response, Option<u64>) = match runtime
        .config
        .sealed_route(&normalized_path)
        .cloned()
    {
        None => (
            (
                StatusCode::NOT_FOUND,
                format!("route `{normalized_path}` is not sealed in `integrity.lock`"),
            )
                .into_response(),
            None,
        ),
        Some(route) => match select_route_target(&route, &headers) {
            Err(error) => (
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "failed to resolve route target for `{}`: {error}",
                        route.path
                    ),
                )
                    .into_response(),
                None,
            ),
            Ok(selected_target) => {
                #[cfg(feature = "websockets")]
                {
                    if selected_target.websocket {
                        let status = if is_websocket_upgrade_request(&headers) {
                            StatusCode::BAD_REQUEST
                        } else {
                            StatusCode::UPGRADE_REQUIRED
                        };
                        match ws {
                            Some(upgrade) => {
                                let websocket_state = state.clone();
                                let websocket_route = route.clone();
                                let websocket_module = selected_target.module.clone();
                                let websocket_target = selected_target.module.clone();
                                (
                                    upgrade
                                        .on_upgrade(move |socket| async move {
                                            if let Err(error) = handle_websocket_connection(
                                                websocket_state,
                                                websocket_route,
                                                websocket_module,
                                                socket,
                                            )
                                            .await
                                            {
                                                tracing::warn!(
                                                    target = %websocket_target,
                                                    "WebSocket session failed: {error:#}"
                                                );
                                            }
                                        })
                                        .into_response(),
                                    None,
                                )
                            }
                            None => (
                                (
                                    status,
                                    format!(
                                        "route `{}` requires a valid WebSocket upgrade request",
                                        route.path
                                    ),
                                )
                                    .into_response(),
                                None,
                            ),
                        }
                    } else if is_websocket_upgrade_request(&headers) {
                        (
                            (
                                StatusCode::BAD_REQUEST,
                                format!(
                                    "route `{}` is not configured for WebSocket upgrades",
                                    route.path
                                ),
                            )
                                .into_response(),
                            None,
                        )
                    } else {
                        match execute_route_with_middleware(
                            &state,
                            &runtime,
                            &route,
                            &headers,
                            &method,
                            &uri,
                            &body,
                            &trailer_fields,
                            hop_limit,
                            Some(&trace_id),
                            sampled_execution,
                            Some(selected_target.module.as_str()),
                        )
                        .await
                        {
                            Ok(result) => {
                                let fuel_consumed = result.fuel_consumed;
                                (guest_response_into_response(result), fuel_consumed)
                            }
                            Err((status, message)) => ((status, message).into_response(), None),
                        }
                    }
                }

                #[cfg(not(feature = "websockets"))]
                {
                    if selected_target.websocket {
                        let status = if is_websocket_upgrade_request(&headers) {
                            StatusCode::NOT_IMPLEMENTED
                        } else {
                            StatusCode::UPGRADE_REQUIRED
                        };
                        (
                            (
                                status,
                                format!(
                                    "route `{}` requires the `websockets` host feature to accept upgraded traffic",
                                    route.path
                                ),
                            )
                                .into_response(),
                            None,
                        )
                    } else if is_websocket_upgrade_request(&headers) {
                        (
                            (
                                StatusCode::BAD_REQUEST,
                                format!(
                                    "route `{}` is not configured for WebSocket upgrades",
                                    route.path
                                ),
                            )
                                .into_response(),
                            None,
                        )
                    } else {
                        match execute_route_with_middleware(
                            &state,
                            &runtime,
                            &route,
                            &headers,
                            &method,
                            &uri,
                            &body,
                            &trailer_fields,
                            hop_limit,
                            Some(&trace_id),
                            sampled_execution,
                            Some(selected_target.module.as_str()),
                        )
                        .await
                        {
                            Ok(result) => {
                                let fuel_consumed = result.fuel_consumed;
                                (guest_response_into_response(result), fuel_consumed)
                            }
                            Err((status, message)) => ((status, message).into_response(), None),
                        }
                    }
                }
            }
        },
    };

    telemetry::record_event(
        &state.telemetry,
        TelemetryEvent::RequestEnd {
            trace_id,
            status: response.status().as_u16(),
            fuel_consumed,
            timestamp: Instant::now(),
        },
    );

    response
}

#[allow(clippy::too_many_arguments)]
async fn execute_route_with_middleware(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<&str>,
    sampled_execution: bool,
    selected_module: Option<&str>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    let invocation = RouteInvocation {
        state: state.clone(),
        runtime: Arc::clone(runtime),
        route: route.clone(),
        headers: headers.clone(),
        method: method.clone(),
        uri: uri.clone(),
        body: body.clone(),
        trailers: trailers.clone(),
        hop_limit,
        trace_id: trace_id.map(str::to_owned),
        sampled_execution,
        selected_module: selected_module.map(str::to_owned),
    };

    resiliency::execute_route_with_resiliency(invocation).await
}

pub(crate) async fn execute_route_with_middleware_inner(
    invocation: &RouteInvocation,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    let state = &invocation.state;
    let runtime = &invocation.runtime;
    let route = &invocation.route;
    let headers = &invocation.headers;
    let method = &invocation.method;
    let uri = &invocation.uri;
    let body = &invocation.body;
    let trailers = &invocation.trailers;
    let hop_limit = invocation.hop_limit;
    let trace_id = invocation.trace_id.as_deref();
    let sampled_execution = invocation.sampled_execution;
    let selected_module = invocation.selected_module.as_deref();
    let mut accumulated_fuel = None;

    if let Some(middleware_name) = route.middleware.as_deref() {
        let middleware_resolved = runtime
            .route_registry
            .resolve_named_route(middleware_name)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let middleware_route = runtime
            .config
            .sealed_route(&middleware_resolved.path)
            .cloned()
            .ok_or_else(|| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "route middleware `{middleware_name}` resolved to missing path `{}`",
                        middleware_resolved.path
                    ),
                )
            })?;
        let middleware_response = execute_route_request(
            state,
            runtime,
            &middleware_route,
            headers,
            method,
            uri,
            body,
            trailers,
            hop_limit,
            trace_id,
            sampled_execution,
            None,
        )
        .await?;
        if middleware_response.response.status != StatusCode::OK {
            return Ok(middleware_response);
        }
        accumulated_fuel = merge_fuel_samples(accumulated_fuel, middleware_response.fuel_consumed);
    }

    let mut result = execute_route_request(
        state,
        runtime,
        route,
        headers,
        method,
        uri,
        body,
        trailers,
        hop_limit,
        trace_id,
        sampled_execution,
        selected_module,
    )
    .await?;
    result.fuel_consumed = merge_fuel_samples(accumulated_fuel, result.fuel_consumed);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn execute_route_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailers: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<&str>,
    sampled_execution: bool,
    selected_module: Option<&str>,
) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
    if route.role == RouteRole::System && should_shed_system_route(&state.telemetry) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("system route `{}` shed under load", route.path),
        ));
    }
    let selected_module = selected_module
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| {
            select_route_module(route, headers)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
        })?;

    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("route `{}` is missing a concurrency limiter", route.path),
            )
        })?;
    match acquire_route_permit(Arc::clone(&semaphore)).await {
        Ok(permit) => {
            execute_route_request_with_acquired_permit(
                state,
                runtime,
                route,
                headers.clone(),
                method.clone(),
                uri.clone(),
                body.clone(),
                trailers.clone(),
                hop_limit,
                trace_id.map(str::to_owned),
                sampled_execution,
                selected_module,
                semaphore,
                permit,
            )
            .await
        }
        Err(RoutePermitError::Closed) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("route `{}` is currently unavailable", route.path),
        )),
        Err(RoutePermitError::TimedOut) => {
            let (receiver, buffered_tier) = state
                .buffered_requests
                .enqueue(BufferedRouteRequest {
                    route_path: route.path.clone(),
                    selected_module,
                    method: method.to_string(),
                    uri: uri.to_string(),
                    headers: header_map_to_guest_fields(headers),
                    body: body.to_vec(),
                    trailers: trailers.clone(),
                    hop_limit: hop_limit.0,
                    trace_id: trace_id.map(str::to_owned),
                    sampled_execution,
                })
                .map_err(|error| {
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!(
                            "route `{}` is saturated and buffering failed: {error}",
                            route.path
                        ),
                    )
                })?;
            match tokio::time::timeout(BUFFER_RESPONSE_WAIT_TIMEOUT, receiver).await {
                Ok(Ok(Ok(mut result))) => {
                    result.response.headers.push((
                        "x-tachyon-buffered".to_owned(),
                        match buffered_tier {
                            BufferedRequestTier::Ram => "ram",
                            BufferedRequestTier::Disk => "disk",
                        }
                        .to_owned(),
                    ));
                    Ok(result)
                }
                Ok(Ok(Err(error))) => Err(error),
                Ok(Err(_)) => Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("route `{}` buffered request was canceled", route.path),
                )),
                Err(_) => Err((
                    StatusCode::SERVICE_UNAVAILABLE,
                    format!("route `{}` buffered request timed out", route.path),
                )),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_route_request_with_acquired_permit(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Bytes,
    trailers: GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: Option<String>,
    sampled_execution: bool,
    selected_module: String,
    semaphore: Arc<RouteExecutionControl>,
    permit: OwnedSemaphorePermit,
) -> BufferedRouteResult {
    let _volume_leases = state
        .volume_manager
        .acquire_route_volumes(route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;
    let _permit = permit;
    let active_request_guard = semaphore.begin_request();
    let propagated_headers = extract_propagated_headers(&headers);
    let engine = if sampled_execution {
        runtime.metered_engine.clone()
    } else {
        runtime.engine.clone()
    };
    let request_config = runtime.config.clone();
    let response_config = runtime.config.clone();
    let route_registry = Arc::clone(&runtime.route_registry);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let storage_broker = Arc::clone(&state.storage_broker);
    let telemetry_context = trace_id.as_ref().map(|trace_id| GuestTelemetryContext {
        handle: state.telemetry.clone(),
        trace_id: trace_id.clone(),
    });
    let runtime_telemetry = state.telemetry.clone();
    let secret_access = SecretAccess::from_route(route, &state.secrets_vault);
    let task_route = route.clone();
    let task_function_name = selected_module.clone();
    let task_propagated_headers = propagated_headers.clone();
    let task_request_headers = headers.clone();
    let task_host_identity = Arc::clone(&state.host_identity);
    let task_async_log_sender = state.async_log_sender.clone();
    #[cfg(feature = "ai-inference")]
    let task_ai_runtime = Arc::clone(&runtime.ai_runtime);
    let guest_request = GuestRequest {
        method: method.to_string(),
        uri: uri.to_string(),
        headers: header_map_to_guest_fields(&headers),
        body: body.clone(),
        trailers: trailers.clone(),
    };
    let result = tokio::task::spawn_blocking(move || {
        execute_guest(
            &engine,
            &task_function_name,
            guest_request,
            &task_route,
            GuestExecutionContext {
                config: request_config,
                sampled_execution,
                runtime_telemetry,
                async_log_sender: task_async_log_sender,
                secret_access,
                request_headers: task_request_headers,
                host_identity: task_host_identity,
                storage_broker,
                telemetry: telemetry_context,
                concurrency_limits,
                propagated_headers: task_propagated_headers,
                #[cfg(feature = "ai-inference")]
                ai_runtime: task_ai_runtime,
            },
        )
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("guest execution task failed: {error}"),
        )
    })?;

    let (response, fuel_consumed) = match result {
        Ok(outcome) => match outcome.output {
            GuestExecutionOutput::Http(response) => (response, outcome.fuel_consumed),
            GuestExecutionOutput::LegacyStdout(stdout) => (
                GuestHttpResponse::new(StatusCode::OK, stdout),
                outcome.fuel_consumed,
            ),
        },
        Err(error) => {
            error.log_if_needed(&selected_module);
            let (status, message) = error.into_response(&response_config);
            return Err((status, message));
        }
    };

    let response = resolve_mesh_response(
        &state.http_client,
        &response_config,
        &route_registry,
        route,
        &state.host_identity,
        &state.uds_fast_path,
        hop_limit,
        &propagated_headers,
        response,
    )
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;

    Ok(RouteExecutionResult {
        response,
        fuel_consumed,
        completion_guard: Some(active_request_guard.into_response_guard()),
    })
}

async fn execute_buffered_route_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    semaphore: Arc<RouteExecutionControl>,
    permit: OwnedSemaphorePermit,
    request: BufferedRouteRequest,
) -> BufferedRouteResult {
    let method = Method::from_bytes(request.method.as_bytes()).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to decode buffered request method: {error}"),
        )
    })?;
    let uri = request.uri.parse::<Uri>().map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to decode buffered request URI: {error}"),
        )
    })?;
    let headers = guest_fields_to_header_map(&request.headers, "buffered request headers")
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    execute_route_request_with_acquired_permit(
        state,
        runtime,
        route,
        headers,
        method,
        uri,
        Bytes::from(request.body),
        request.trailers,
        HopLimit(request.hop_limit),
        request.trace_id,
        request.sampled_execution,
        request.selected_module,
        semaphore,
        permit,
    )
    .await
}

async fn acquire_route_permit(
    control: Arc<RouteExecutionControl>,
) -> std::result::Result<OwnedSemaphorePermit, RoutePermitError> {
    match Arc::clone(&control.semaphore).try_acquire_owned() {
        Ok(permit) => Ok(permit),
        Err(TryAcquireError::Closed) => Err(RoutePermitError::Closed),
        Err(TryAcquireError::NoPermits) => {
            control.pending_waiters.fetch_add(1, Ordering::SeqCst);
            let result = tokio::time::timeout(
                ROUTE_CONCURRENCY_WAIT_TIMEOUT,
                Arc::clone(&control.semaphore).acquire_owned(),
            )
            .await;
            control.pending_waiters.fetch_sub(1, Ordering::SeqCst);

            match result {
                Ok(Ok(permit)) => Ok(permit),
                Ok(Err(_)) => Err(RoutePermitError::Closed),
                Err(_) => Err(RoutePermitError::TimedOut),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_mesh_response(
    http_client: &Client,
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    host_identity: &HostIdentity,
    uds_fast_path: &UdsFastPathRegistry,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    response: GuestHttpResponse,
) -> std::result::Result<GuestHttpResponse, String> {
    let Some(target) = extract_mesh_fetch_url(&response.body) else {
        return Ok(response);
    };
    let url = resolve_mesh_fetch_target(config, route_registry, caller_route, target)?;
    let inject_identity = is_internal_mesh_target(target);
    let identity_token = if inject_identity {
        Some(
            host_identity
                .sign_route(caller_route)
                .map_err(|error| format!("failed to sign mesh caller identity: {error:#}"))?,
        )
    } else {
        None
    };
    let response = send_mesh_fetch_request(
        http_client,
        uds_fast_path,
        &url,
        hop_limit,
        propagated_headers,
        identity_token.as_deref(),
    )
    .await?;

    let status = response.status();
    let headers = header_map_to_guest_fields(response.headers());
    let body = response.bytes().await.map_err(|error| {
        format!("failed to read mesh fetch response body from `{url}`: {error}")
    })?;

    if status == StatusCode::LOOP_DETECTED || status.is_success() {
        Ok(GuestHttpResponse {
            status,
            headers,
            body,
            trailers: Vec::new(),
        })
    } else {
        Err(format!(
            "mesh fetch to `{url}` returned an error status: {status}"
        ))
    }
}

fn apply_mesh_fetch_headers(
    mut request: reqwest::RequestBuilder,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    identity_token: Option<&str>,
) -> reqwest::RequestBuilder {
    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    for header in propagated_headers
        .iter()
        .filter(|header| !header.name.eq_ignore_ascii_case(TACHYON_IDENTITY_HEADER))
    {
        request = request.header(&header.name, &header.value);
    }
    if let Some(identity_token) = identity_token {
        request = request.header(TACHYON_IDENTITY_HEADER, format!("Bearer {identity_token}"));
    }

    request
}

async fn send_mesh_fetch_request(
    http_client: &Client,
    _uds_fast_path: &UdsFastPathRegistry,
    url: &str,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    identity_token: Option<&str>,
) -> std::result::Result<reqwest::Response, String> {
    #[cfg(unix)]
    if let Some(peer) = _uds_fast_path.discover_peer_for_url(url) {
        let uds_client = Client::builder()
            .unix_socket(peer.socket_path.as_path())
            .build()
            .map_err(|error| {
                format!(
                    "failed to build UDS mesh client for `{}`: {error}",
                    peer.socket_path.display()
                )
            })?;
        let request = apply_mesh_fetch_headers(
            uds_client.get(url),
            hop_limit,
            propagated_headers,
            identity_token,
        );
        match request.send().await {
            Ok(response) => return Ok(response),
            Err(error) => {
                _uds_fast_path.note_connect_failure(&peer);
                tracing::debug!(
                    socket = %peer.socket_path.display(),
                    url = %url,
                    "UDS fast-path unavailable, falling back to TCP: {error}"
                );
            }
        }
    }

    apply_mesh_fetch_headers(
        http_client.get(url),
        hop_limit,
        propagated_headers,
        identity_token,
    )
    .send()
    .await
    .map_err(|error| format!("mesh fetch to `{url}` failed: {error}"))
}

fn extract_mesh_fetch_url(stdout: &Bytes) -> Option<&str> {
    std::str::from_utf8(stdout)
        .ok()?
        .trim()
        .strip_prefix("MESH_FETCH:")
        .map(str::trim)
        .filter(|url| !url.is_empty())
}

fn is_internal_mesh_target(target: &str) -> bool {
    if target.starts_with('/') {
        return true;
    }

    (target.starts_with("http://") || target.starts_with("https://"))
        && reqwest::Url::parse(target)
            .ok()
            .and_then(|url| url.host_str().map(is_internal_mesh_host))
            .unwrap_or(false)
}

fn select_route_module(
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> std::result::Result<String, String> {
    select_route_target_with_roll(route, headers, None).map(|target| target.module)
}

fn select_stream_route_module(route: &IntegrityRoute) -> std::result::Result<String, String> {
    if route.targets.is_empty() {
        return Ok(route.name.clone());
    }

    select_route_target_with_roll(route, &HeaderMap::new(), None)
        .map(|target| target.module)
        .or_else(|_| Ok(route.name.clone()))
}

fn select_route_target(
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> std::result::Result<SelectedRouteTarget, String> {
    select_route_target_with_roll(route, headers, None)
}

fn select_route_target_with_roll(
    route: &IntegrityRoute,
    headers: &HeaderMap,
    random_roll: Option<u64>,
) -> std::result::Result<SelectedRouteTarget, String> {
    if route.targets.is_empty() {
        return Ok(SelectedRouteTarget {
            module: route.name.clone(),
            websocket: false,
        });
    }

    for target in &route.targets {
        if target
            .match_header
            .as_ref()
            .is_some_and(|matcher| request_header_matches(headers, matcher))
        {
            return Ok(SelectedRouteTarget {
                module: target.module.clone(),
                websocket: target.websocket,
            });
        }
    }

    let total_weight = route
        .targets
        .iter()
        .map(|target| u64::from(target.weight))
        .sum::<u64>();
    if total_weight > 0 {
        let draw = match random_roll {
            Some(roll) => roll % total_weight,
            None => rand::thread_rng().gen_range(0..total_weight),
        };
        let mut cumulative_weight = 0_u64;
        for target in &route.targets {
            if target.weight == 0 {
                continue;
            }
            cumulative_weight = cumulative_weight.saturating_add(u64::from(target.weight));
            if draw < cumulative_weight {
                return Ok(SelectedRouteTarget {
                    module: target.module.clone(),
                    websocket: target.websocket,
                });
            }
        }
    }

    resolve_function_name(&route.path)
        .map(|module| SelectedRouteTarget {
            module,
            websocket: false,
        })
        .ok_or_else(|| {
            format!(
                "route `{}` does not define a routable guest target",
                route.path
            )
        })
}

fn request_header_matches(headers: &HeaderMap, matcher: &HeaderMatch) -> bool {
    headers
        .get(matcher.name.as_str())
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .is_some_and(|value| value == matcher.value)
}

fn is_websocket_upgrade_request(headers: &HeaderMap) -> bool {
    let connection_upgrade = headers
        .get("connection")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|segment| segment.eq_ignore_ascii_case("upgrade"))
        })
        .unwrap_or(false);
    let websocket_upgrade = headers
        .get("upgrade")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);

    connection_upgrade && websocket_upgrade
}

fn extract_propagated_headers(headers: &HeaderMap) -> Vec<PropagatedHeader> {
    let Some(value) = headers
        .get(TACHYON_COHORT_HEADER)
        .or_else(|| headers.get(COHORT_HEADER))
        .and_then(|header| header.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Vec::new();
    };

    vec![
        PropagatedHeader {
            name: COHORT_HEADER.to_owned(),
            value: value.to_owned(),
        },
        PropagatedHeader {
            name: TACHYON_COHORT_HEADER.to_owned(),
            value: value.to_owned(),
        },
    ]
}

fn resolve_incoming_hop_limit(headers: &HeaderMap) -> std::result::Result<HopLimit, ()> {
    let hop_limit = headers
        .get(HOP_LIMIT_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(DEFAULT_HOP_LIMIT);

    if hop_limit == 0 {
        Err(())
    } else {
        Ok(HopLimit(hop_limit))
    }
}

fn resolve_mesh_fetch_target(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    target: &str,
) -> std::result::Result<String, String> {
    if target.starts_with('/') {
        return Ok(format!("{}{}", internal_mesh_base_url(config)?, target));
    }

    if target.starts_with("http://") || target.starts_with("https://") {
        let url = reqwest::Url::parse(target)
            .map_err(|error| format!("mesh fetch target `{target}` is not a valid URL: {error}"))?;

        if !url.host_str().is_some_and(is_internal_mesh_host) {
            return Ok(target.to_owned());
        }

        let normalized_path = normalize_route_path(url.path());
        let base_url = internal_mesh_base_url(config)?;
        if route_registry.by_path.contains_key(&normalized_path) {
            return Ok(format!(
                "{base_url}{}",
                append_query(&normalized_path, url.query())
            ));
        }

        let dependency_segments = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if dependency_segments.len() != 1 {
            return Err(format!(
                "internal mesh target `{target}` must identify a sealed route path or a single dependency name"
            ));
        }
        let dependency_name = dependency_segments[0];
        let resolved_route =
            route_registry.resolve_dependency_route(&caller_route.path, dependency_name)?;
        return Ok(format!(
            "{base_url}{}",
            append_query(&resolved_route.path, url.query())
        ));
    }

    Err(format!(
        "mesh fetch target `{target}` must be an absolute URL or an absolute route path"
    ))
}

fn is_internal_mesh_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("tachyon") || host.eq_ignore_ascii_case("mesh")
}

fn append_query(path: &str, query: Option<&str>) -> String {
    match query {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path.to_owned(),
    }
}

fn internal_mesh_base_url(config: &IntegrityConfig) -> std::result::Result<String, String> {
    let host_address = config.host_address.trim();
    if host_address.is_empty() {
        return Err(
            "mesh fetch cannot resolve a relative route without a configured host address"
                .to_owned(),
        );
    }

    if let Ok(socket_addr) = host_address.parse::<SocketAddr>() {
        return Ok(format!(
            "http://{}:{}",
            client_connect_host(socket_addr.ip()),
            socket_addr.port()
        ));
    }

    Ok(format!("http://{}", host_address.trim_end_matches('/')))
}

impl RouteRegistry {
    fn build(config: &IntegrityConfig) -> Result<Self> {
        let mut registry = Self::default();
        let mut seen_versions = HashMap::<(String, String), String>::new();

        for route in &config.routes {
            let version = Version::parse(route.version.trim()).with_context(|| {
                format!(
                    "Integrity Validation Failed: route `{}` has invalid semantic version `{}`",
                    route.path, route.version
                )
            })?;
            let dependencies = route
                .dependencies
                .iter()
                .map(|(name, requirement)| {
                    VersionReq::parse(requirement.trim())
                        .map(|parsed| (name.clone(), parsed))
                        .map_err(|error| {
                            anyhow!(
                                "Integrity Validation Failed: route `{}` has invalid dependency requirement `{}` for `{}`: {}",
                                route.path,
                                requirement,
                                name,
                                error
                            )
                        })
                })
                .collect::<Result<HashMap<_, _>>>()?;

            let resolved = ResolvedRoute {
                path: route.path.clone(),
                name: route.name.clone(),
                version,
                dependencies,
                requires_credentials: route.requires_credentials.iter().cloned().collect(),
            };
            let version_text = resolved.version.to_string();
            if let Some(existing_path) = seen_versions.insert(
                (resolved.name.clone(), version_text.clone()),
                resolved.path.clone(),
            ) {
                return Err(anyhow!(
                    "Integrity Validation Failed: routes `{}` and `{}` both declare `{}` version `{}`",
                    existing_path,
                    resolved.path,
                    resolved.name,
                    version_text
                ));
            }

            registry
                .by_name
                .entry(resolved.name.clone())
                .or_default()
                .push(resolved.clone());
            registry.by_path.insert(resolved.path.clone(), resolved);
        }

        for routes in registry.by_name.values_mut() {
            routes.sort_by(|left, right| {
                right
                    .version
                    .cmp(&left.version)
                    .then_with(|| left.path.cmp(&right.path))
            });
        }

        for route in registry.by_path.values() {
            registry
                .ensure_dependencies_satisfied(route)
                .map_err(anyhow::Error::msg)?;
        }

        for route in &config.routes {
            if let Some(middleware) = &route.middleware {
                let resolved_middleware = registry
                    .resolve_named_route(middleware)
                    .map_err(anyhow::Error::msg)?;
                if resolved_middleware.path == route.path {
                    return Err(anyhow!(
                        "Integrity Validation Failed: route `{}` cannot use itself (`{}`) as middleware",
                        route.path,
                        middleware
                    ));
                }
            }
        }

        Ok(registry)
    }

    fn ensure_dependencies_satisfied(
        &self,
        route: &ResolvedRoute,
    ) -> std::result::Result<(), String> {
        for (dependency_name, requirement) in &route.dependencies {
            let dependency =
                self.resolve_dependency_candidate(route, dependency_name, requirement)?;
            let missing_credentials = dependency
                .requires_credentials
                .difference(&route.requires_credentials)
                .cloned()
                .collect::<Vec<_>>();

            if !missing_credentials.is_empty() {
                return Err(format!(
                    "Credential delegation failed: route {} ({}@{}) must also declare {:?} to satisfy dependency {} ({}@{})",
                    route.path,
                    route.name,
                    route.version,
                    missing_credentials,
                    dependency.path,
                    dependency.name,
                    dependency.version
                ));
            }
        }

        Ok(())
    }

    fn resolve_dependency_route(
        &self,
        caller_path: &str,
        dependency_name: &str,
    ) -> std::result::Result<&ResolvedRoute, String> {
        let caller = self.by_path.get(caller_path).ok_or_else(|| {
            format!(
                "mesh fetch caller route `{caller_path}` is missing from the sealed dependency registry"
            )
        })?;
        let requirement = caller.dependencies.get(dependency_name).ok_or_else(|| {
            format!(
                "route {} ({}@{}) does not declare `{}` in its sealed dependencies",
                caller.path, caller.name, caller.version, dependency_name
            )
        })?;

        self.resolve_dependency_candidate(caller, dependency_name, requirement)
    }

    fn resolve_named_route(&self, route_name: &str) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(route_name)
            .and_then(|routes| routes.first())
            .ok_or_else(|| {
                format!("route middleware `{route_name}` does not match any sealed route name")
            })
    }

    fn resolve_dependency_candidate(
        &self,
        caller: &ResolvedRoute,
        dependency_name: &str,
        requirement: &VersionReq,
    ) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(dependency_name)
            .into_iter()
            .flatten()
            .find(|candidate| requirement.matches(&candidate.version))
            .ok_or_else(|| {
                format!(
                    "Dependency resolution failed: route {} ({}@{}) requires {} matching {}, but no compatible version was loaded",
                    caller.path,
                    caller.name,
                    caller.version,
                    dependency_name,
                    requirement
                )
            })
    }
}

impl BatchTargetRegistry {
    fn build(config: &IntegrityConfig) -> Result<Self> {
        let mut registry = Self::default();
        for target in &config.batch_targets {
            if registry
                .by_name
                .insert(target.name.clone(), target.clone())
                .is_some()
            {
                return Err(anyhow!(
                    "Integrity Validation Failed: batch target `{}` is defined more than once",
                    target.name
                ));
            }
        }

        Ok(registry)
    }

    fn get(&self, name: &str) -> Option<&IntegrityBatchTarget> {
        self.by_name.get(name)
    }
}

fn client_connect_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("[{}]", Ipv6Addr::LOCALHOST),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(unix)]
fn discovery_publish_ip(config: &IntegrityConfig) -> Result<String> {
    let host_address = config.host_address.trim();
    if host_address.is_empty() {
        return Err(anyhow!(
            "cannot publish a UDS fast-path endpoint without a configured host address"
        ));
    }

    if let Ok(socket_addr) = host_address.parse::<SocketAddr>() {
        return Ok(match socket_addr.ip() {
            IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
            IpAddr::V4(ip) => ip.to_string(),
            IpAddr::V6(ip) if ip.is_unspecified() => Ipv6Addr::LOCALHOST.to_string(),
            IpAddr::V6(ip) => ip.to_string(),
        });
    }

    let host = host_address
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or(host_address)
        .split(':')
        .next()
        .unwrap_or(host_address)
        .trim_matches('[')
        .trim_matches(']');
    if host.is_empty() {
        return Err(anyhow!(
            "cannot derive a publishable IP from host address `{host_address}`"
        ));
    }

    Ok(host.to_owned())
}

fn loop_detected_response() -> Response {
    (
        StatusCode::LOOP_DETECTED,
        "Tachyon Mesh: Routing loop detected (Hop limit exceeded)",
    )
        .into_response()
}

impl HopLimit {
    fn as_header_value(self) -> HeaderValue {
        HeaderValue::from_str(&self.0.to_string())
            .expect("hop limit should always produce a valid header value")
    }

    fn decremented(self) -> u32 {
        self.0.saturating_sub(1)
    }
}

fn execute_guest(
    engine: &Engine,
    function_name: &str,
    request: GuestRequest,
    route: &IntegrityRoute,
    execution: GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    #[cfg(not(feature = "ai-inference"))]
    if requires_ai_inference_feature(function_name) {
        return Err(ExecutionError::Internal(format!(
            "guest `{function_name}` requires `core-host` to be built with `--features ai-inference`"
        )));
    }

    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let cache_scope = if execution.sampled_execution {
        "metered"
    } else {
        "default"
    };

    if let Ok(component) = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        cache_scope,
    ) {
        let component_result = match route.role {
            RouteRole::User => execute_component_guest(
                engine,
                request.clone(),
                route,
                &module_path,
                &component,
                &execution,
            ),
            RouteRole::System => execute_system_component_guest(
                engine,
                request.clone(),
                route,
                &module_path,
                &component,
                &execution,
            ),
        };

        match component_result {
            Ok(response) => return Ok(response),
            Err(ExecutionError::Internal(message))
                if message.contains("no exported instance named `tachyon:mesh/handler`") => {}
            Err(error) => return Err(error),
        }
    }

    let (module_path, module) = resolve_legacy_guest_module(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        cache_scope,
    )?;

    execute_legacy_guest(
        engine,
        function_name,
        request.body,
        route,
        &module_path,
        module,
        &execution,
    )
}

#[derive(Clone, Copy)]
enum CompiledArtifactKind {
    Component,
    Module,
}

fn load_component_with_core_store(
    engine: &Engine,
    module_path: &Path,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> Result<Component> {
    let wasm_bytes = fs::read(module_path).with_context(|| {
        format!(
            "failed to read guest component artifact from {}",
            module_path.display()
        )
    })?;
    let cache_key = compiled_artifact_cache_key(
        module_path,
        &wasm_bytes,
        CompiledArtifactKind::Component,
        cache_scope,
    );

    if let Some(cached) = core_store
        .get(store::CoreStoreBucket::CwasmCache, &cache_key)
        .with_context(|| {
            format!(
                "failed to read cached component `{}`",
                module_path.display()
            )
        })?
    {
        // SAFETY: cached bytes originate from Engine::precompile_component for this host.
        if let Ok(component) = unsafe { Component::deserialize(engine, &cached) } {
            return Ok(component);
        }
    }

    let compiled = engine.precompile_component(&wasm_bytes).map_err(|error| {
        anyhow!(
            "failed to precompile guest component artifact from {}: {error}",
            module_path.display()
        )
    })?;
    core_store
        .put(store::CoreStoreBucket::CwasmCache, &cache_key, &compiled)
        .with_context(|| format!("failed to cache component `{}`", module_path.display()))?;
    // SAFETY: compiled bytes were produced by Engine::precompile_component above.
    unsafe { Component::deserialize(engine, &compiled) }.map_err(|error| {
        anyhow!(
            "failed to deserialize cached guest component from {}: {error}",
            module_path.display()
        )
    })
}

fn resolve_legacy_guest_module(
    engine: &Engine,
    function_name: &str,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> std::result::Result<(PathBuf, Module), ExecutionError> {
    let candidates = guest_module_candidate_paths(function_name);
    let candidate_strings = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let mut last_error = None;

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }

        match load_module_with_core_store(engine, &candidate, core_store, cache_scope) {
            Ok(module) => return Ok((normalize_path(candidate), module)),
            Err(error) => last_error = Some((normalize_path(candidate), error)),
        }
    }

    if let Some((path, error)) = last_error {
        return Err(ExecutionError::Internal(format!(
            "failed to load guest artifact from {}: {error:#}",
            path.display()
        )));
    }

    Err(ExecutionError::GuestModuleNotFound(
        GuestModuleNotFound::new(function_name, format_candidate_list(&candidate_strings)),
    ))
}

fn load_module_with_core_store(
    engine: &Engine,
    module_path: &Path,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> Result<Module> {
    let wasm_bytes = fs::read(module_path).with_context(|| {
        format!(
            "failed to read guest module artifact from {}",
            module_path.display()
        )
    })?;
    let cache_key = compiled_artifact_cache_key(
        module_path,
        &wasm_bytes,
        CompiledArtifactKind::Module,
        cache_scope,
    );

    if let Some(cached) = core_store
        .get(store::CoreStoreBucket::CwasmCache, &cache_key)
        .with_context(|| format!("failed to read cached module `{}`", module_path.display()))?
    {
        // SAFETY: cached bytes originate from Engine::precompile_module for this host.
        if let Ok(module) = unsafe { Module::deserialize(engine, &cached) } {
            return Ok(module);
        }
    }

    let compiled = engine.precompile_module(&wasm_bytes).map_err(|error| {
        anyhow!(
            "failed to precompile guest module artifact from {}: {error}",
            module_path.display()
        )
    })?;
    core_store
        .put(store::CoreStoreBucket::CwasmCache, &cache_key, &compiled)
        .with_context(|| format!("failed to cache module `{}`", module_path.display()))?;
    // SAFETY: compiled bytes were produced by Engine::precompile_module above.
    unsafe { Module::deserialize(engine, &compiled) }.map_err(|error| {
        anyhow!(
            "failed to deserialize cached guest module from {}: {error}",
            module_path.display()
        )
    })
}

fn compiled_artifact_cache_key(
    module_path: &Path,
    wasm_bytes: &[u8],
    kind: CompiledArtifactKind,
    cache_scope: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(wasm_bytes);
    let digest = hasher.finalize();
    let kind = match kind {
        CompiledArtifactKind::Component => "component",
        CompiledArtifactKind::Module => "module",
    };

    format!(
        "{kind}:{cache_scope}:{}:{}",
        module_path.display(),
        hex::encode(digest)
    )
}

fn execute_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to component linker",
        )
    })?;
    #[cfg(feature = "ai-inference")]
    add_accelerator_interfaces_to_component_linker(
        &mut linker,
        execution.ai_runtime.as_ref(),
        "component linker",
    )?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    #[cfg(feature = "ai-inference")]
    {
        store.data_mut().ai_runtime = Some(Arc::clone(&execution.ai_runtime));
    }
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

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
    record_wasm_start(execution.telemetry.as_ref());
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            headers: request.headers,
            body: request.body.to_vec(),
            trailers: request.trailers,
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::Http(GuestHttpResponse {
            status,
            headers: response.headers,
            body: Bytes::from(response.body),
            trailers: response.trailers,
        }),
        fuel_consumed,
    })
}

fn execute_udp_layer4_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    function_name: &str,
    source: SocketAddr,
    payload: Bytes,
    execution: &GuestExecutionContext,
) -> std::result::Result<Vec<UdpResponseDatagram>, ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let component = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        "default",
    )
    .map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to load UDP guest component from {}: {error:#}",
            module_path.display()
        ))
    })?;

    execute_udp_component_guest(
        engine,
        route,
        &module_path,
        &component,
        source,
        payload,
        execution,
    )
}

fn execute_udp_component_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    source: SocketAddr,
    payload: Bytes,
    execution: &GuestExecutionContext,
) -> std::result::Result<Vec<UdpResponseDatagram>, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to UDP component linker",
        )
    })?;
    udp_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to UDP component linker",
        )
    })?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let bindings = udp_component_bindings::UdpFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        let message = format!(
            "failed to instantiate UDP guest component from {}",
            component_path.display()
        );
        let error_message = error.to_string();
        if error_message.contains("no exported instance named `tachyon:mesh/udp-handler`") {
            ExecutionError::Internal(format!(
                "guest component `{}` does not export the UDP packet handler",
                component_path.display()
            ))
        } else {
            guest_execution_error(error, message)
        }
    })?;
    record_wasm_start(execution.telemetry.as_ref());
    let source_ip = source.ip().to_string();
    let response = bindings.tachyon_mesh_udp_handler().call_handle_packet(
        &mut store,
        &source_ip,
        source.port(),
        payload.as_ref(),
    );
    record_wasm_end(execution.telemetry.as_ref());
    let _fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "guest UDP component `handle-packet` trapped")
    })?;

    response
        .into_iter()
        .map(|datagram| {
            let target_ip = datagram.target_ip.parse::<IpAddr>().map_err(|error| {
                ExecutionError::Internal(format!(
                    "guest UDP component returned an invalid target IP `{}`: {error}",
                    datagram.target_ip
                ))
            })?;
            Ok(UdpResponseDatagram {
                target: SocketAddr::new(target_ip, datagram.target_port),
                payload: Bytes::from(datagram.payload),
            })
        })
        .collect()
}

#[cfg(feature = "websockets")]
fn execute_websocket_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    function_name: &str,
    incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let component = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        "default",
    )
    .map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to load WebSocket guest component from {}: {error:#}",
            module_path.display()
        ))
    })?;

    execute_websocket_component_guest(
        engine,
        route,
        &module_path,
        &component,
        incoming,
        outgoing,
        execution,
    )
}

#[cfg(feature = "websockets")]
fn execute_websocket_component_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::websocket::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WebSocket host functions to component linker",
        )
    })?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let stored_connection = store
        .data_mut()
        .table
        .push(HostWebSocketConnection { incoming, outgoing })
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to store WebSocket connection resource for {}: {error}",
                component_path.display()
            ))
        })?;
    let connection = wasmtime::component::Resource::<
        websocket_component_bindings::tachyon::mesh::websocket::Connection,
    >::new_own(stored_connection.rep());

    let bindings = websocket_component_bindings::WebsocketFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        let message = format!(
            "failed to instantiate WebSocket guest component from {}",
            component_path.display()
        );
        let error_message = error.to_string();
        if error_message.contains("on-connect") {
            ExecutionError::Internal(format!(
                "guest component `{}` does not export the WebSocket `on-connect` handler",
                component_path.display()
            ))
        } else {
            guest_execution_error(error, message)
        }
    })?;
    record_wasm_start(execution.telemetry.as_ref());
    let result = bindings.call_on_connect(&mut store, connection);
    record_wasm_end(execution.telemetry.as_ref());
    let _fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    result.map_err(|error| {
        guest_execution_error(error, "guest WebSocket component `on-connect` trapped")
    })?;
    Ok(())
}

#[cfg(feature = "websockets")]
fn websocket_message_to_host_frame(message: AxumWebSocketMessage) -> HostWebSocketFrame {
    match message {
        AxumWebSocketMessage::Text(text) => HostWebSocketFrame::Text(text.to_string()),
        AxumWebSocketMessage::Binary(bytes) => HostWebSocketFrame::Binary(bytes.to_vec()),
        AxumWebSocketMessage::Ping(bytes) => HostWebSocketFrame::Ping(bytes.to_vec()),
        AxumWebSocketMessage::Pong(bytes) => HostWebSocketFrame::Pong(bytes.to_vec()),
        AxumWebSocketMessage::Close(_) => HostWebSocketFrame::Close,
    }
}

#[cfg(feature = "websockets")]
fn host_frame_to_websocket_message(frame: HostWebSocketFrame) -> AxumWebSocketMessage {
    match frame {
        HostWebSocketFrame::Text(text) => AxumWebSocketMessage::Text(text),
        HostWebSocketFrame::Binary(bytes) => AxumWebSocketMessage::Binary(bytes),
        HostWebSocketFrame::Ping(bytes) => AxumWebSocketMessage::Ping(bytes),
        HostWebSocketFrame::Pong(bytes) => AxumWebSocketMessage::Pong(bytes),
        HostWebSocketFrame::Close => AxumWebSocketMessage::Close(None),
    }
}

#[cfg(feature = "websockets")]
fn websocket_binding_frame_to_host_frame(
    frame: websocket_component_bindings::tachyon::mesh::websocket::Frame,
) -> HostWebSocketFrame {
    match frame {
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Text(text) => {
            HostWebSocketFrame::Text(text)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Binary(bytes) => {
            HostWebSocketFrame::Binary(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Ping(bytes) => {
            HostWebSocketFrame::Ping(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Pong(bytes) => {
            HostWebSocketFrame::Pong(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Close => {
            HostWebSocketFrame::Close
        }
    }
}

#[cfg(feature = "websockets")]
fn host_frame_to_websocket_binding_frame(
    frame: HostWebSocketFrame,
) -> websocket_component_bindings::tachyon::mesh::websocket::Frame {
    match frame {
        HostWebSocketFrame::Text(text) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Text(text)
        }
        HostWebSocketFrame::Binary(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Binary(bytes)
        }
        HostWebSocketFrame::Ping(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Ping(bytes)
        }
        HostWebSocketFrame::Pong(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Pong(bytes)
        }
        HostWebSocketFrame::Close => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Close
        }
    }
}

fn execute_system_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::storage_broker::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add storage broker functions to system component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let bindings =
        system_component_bindings::SystemFaasGuest::instantiate(&mut store, component, &linker)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to instantiate system guest component from {}",
                        component_path.display()
                    ),
                )
            })?;
    record_wasm_start(execution.telemetry.as_ref());
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &system_component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            headers: request.headers,
            body: request.body.to_vec(),
            trailers: request.trailers,
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "system guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "system guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::Http(GuestHttpResponse {
            status,
            headers: response.headers,
            body: Bytes::from(response.body),
            trailers: response.trailers,
        }),
        fuel_consumed,
    })
}

impl BackgroundTickRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        engine: &Engine,
        config: &IntegrityConfig,
        route: &IntegrityRoute,
        function_name: &str,
        telemetry: TelemetryHandle,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<Self, ExecutionError> {
        let module_path = resolve_guest_module_path(function_name)
            .map_err(ExecutionError::GuestModuleNotFound)?;
        let component = Component::from_file(engine, &module_path).map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to load background system component from {}",
                    module_path.display()
                ),
            )
        })?;

        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
            guest_execution_error(
                error,
                "failed to add WASI preview2 functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add scaling metrics functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add outbound HTTP functions to background component linker",
            )
        })?;

        let mut store = Store::new(
            engine,
            ComponentHostState::new(
                route,
                config.clone(),
                config.guest_memory_limit_bytes,
                telemetry,
                SecretAccess::default(),
                HeaderMap::new(),
                host_identity,
                storage_broker,
                concurrency_limits,
                Vec::new(),
            )?,
        );
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(config.guest_fuel_budget)
            .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;

        let bindings = background_component_bindings::BackgroundSystemFaas::instantiate(
            &mut store, &component, &linker,
        )
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate background system component from {}",
                    module_path.display()
                ),
            )
        })?;

        Ok(Self {
            function_name: function_name.to_owned(),
            route_path: route.path.clone(),
            store,
            bindings,
        })
    }

    fn tick(&mut self) -> std::result::Result<(), ExecutionError> {
        self.bindings
            .call_on_tick(&mut self.store)
            .map_err(|error| {
                guest_execution_error(error, "background system guest `on-tick` trapped")
            })
    }
}

fn execute_legacy_guest(
    engine: &Engine,
    function_name: &str,
    body: Bytes,
    route: &IntegrityRoute,
    module_path: &Path,
    module: Module,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let linker = build_linker(engine)?;
    let stdin_file = create_guest_stdin_file(&body)?;
    let stdout_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stdout,
        execution.async_log_sender.clone(),
        true,
        execution.config.max_stdout_bytes,
    );
    let stderr_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stderr,
        execution.async_log_sender.clone(),
        false,
        0,
    );
    let mut wasi = WasiCtxBuilder::new();
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(InputFile::new(stdin_file.file.try_clone().map_err(
            |error| guest_execution_error(error.into(), "failed to clone guest stdin file handle"),
        )?))
        .stdout(stdout_capture.clone())
        .stderr(stderr_capture);
    add_route_environment(&mut wasi, route, execution.host_identity.as_ref())?;

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

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(
            wasi,
            execution.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&execution.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;
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

    record_wasm_start(execution.telemetry.as_ref());
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    handle_guest_entrypoint_result(entrypoint_name, call_result)?;
    let stdout_bytes = stdout_capture.finish()?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(stdout_bytes),
        fuel_consumed,
    })
}

fn execute_legacy_guest_with_stdio(
    engine: &Engine,
    route: &IntegrityRoute,
    module_path: &Path,
    module: Module,
    execution: &GuestExecutionContext,
    stdin: impl StdinStream + 'static,
    stdout: impl StdoutStream + 'static,
) -> std::result::Result<(), ExecutionError> {
    let linker = build_linker(engine)?;
    let mut wasi = WasiCtxBuilder::new();
    add_route_environment(&mut wasi, route, execution.host_identity.as_ref())?;
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(stdin)
        .stdout(stdout);

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

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(
            wasi,
            execution.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&execution.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;
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

    record_wasm_start(execution.telemetry.as_ref());
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(execution.telemetry.as_ref());
    let _ = sampled_fuel_consumed(&mut store, execution)?;
    handle_guest_entrypoint_result(entrypoint_name, call_result)?;
    Ok(())
}

#[derive(Clone)]
struct TcpSocketStdin {
    socket: Arc<Mutex<std::net::TcpStream>>,
}

impl TcpSocketStdin {
    fn new(socket: std::net::TcpStream) -> Self {
        Self {
            socket: Arc::new(Mutex::new(socket)),
        }
    }
}

impl IsTerminal for TcpSocketStdin {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdinStream for TcpSocketStdin {
    fn p2_stream(&self) -> Box<dyn InputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncRead + Send + Sync> {
        Box::new(tokio::io::empty())
    }
}

#[async_trait::async_trait]
impl InputStream for TcpSocketStdin {
    fn read(&mut self, size: usize) -> StreamResult<Bytes> {
        if size == 0 {
            return Ok(Bytes::new());
        }

        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdin socket lock poisoned"))?;
        let mut buffer = vec![0_u8; size];
        loop {
            match std::io::Read::read(&mut *socket, &mut buffer) {
                Ok(0) => return Err(StreamError::Closed),
                Ok(read) => {
                    buffer.truncate(read);
                    return Ok(Bytes::from(buffer));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(StreamError::LastOperationFailed(error.into())),
            }
        }
    }
}

#[async_trait::async_trait]
impl Pollable for TcpSocketStdin {
    async fn ready(&mut self) {}
}

#[derive(Clone)]
struct TcpSocketStdout {
    socket: Arc<Mutex<std::net::TcpStream>>,
}

impl TcpSocketStdout {
    fn new(socket: std::net::TcpStream) -> Self {
        Self {
            socket: Arc::new(Mutex::new(socket)),
        }
    }
}

impl IsTerminal for TcpSocketStdout {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for TcpSocketStdout {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(tokio::io::sink())
    }
}

#[async_trait::async_trait]
impl OutputStream for TcpSocketStdout {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdout socket lock poisoned"))?;
        loop {
            match std::io::Write::write_all(&mut *socket, &bytes) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(StreamError::LastOperationFailed(error.into())),
            }
        }
    }

    fn flush(&mut self) -> StreamResult<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdout socket lock poisoned"))?;
        std::io::Write::flush(&mut *socket)
            .map_err(|error| StreamError::LastOperationFailed(error.into()))
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(4096)
    }
}

#[async_trait::async_trait]
impl Pollable for TcpSocketStdout {
    async fn ready(&mut self) {}
}

#[derive(Default)]
struct AsyncGuestOutputState {
    function_name: String,
    capture_response: bool,
    max_response_bytes: usize,
    response: Vec<u8>,
    pending: Vec<u8>,
    response_overflowed: bool,
    sender: Option<mpsc::Sender<AsyncLogEntry>>,
    stream_type: Option<GuestLogStreamType>,
}

#[derive(Clone, Default)]
struct AsyncGuestOutputCapture {
    state: Arc<Mutex<AsyncGuestOutputState>>,
}

impl AsyncGuestOutputCapture {
    fn new(
        function_name: impl Into<String>,
        stream_type: GuestLogStreamType,
        sender: mpsc::Sender<AsyncLogEntry>,
        capture_response: bool,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(AsyncGuestOutputState {
                function_name: function_name.into(),
                capture_response,
                max_response_bytes,
                response: Vec::new(),
                pending: Vec::new(),
                response_overflowed: false,
                sender: Some(sender),
                stream_type: Some(stream_type),
            })),
        }
    }

    fn finish(&self) -> std::result::Result<Bytes, ExecutionError> {
        let mut state = self.state.lock().map_err(|_| {
            ExecutionError::Internal("guest async stdout capture lock poisoned".to_owned())
        })?;
        flush_async_guest_output(&mut state);
        if state.response_overflowed {
            return Err(ExecutionError::ResourceLimitExceeded {
                kind: ResourceLimitKind::Stdout,
                detail: format!(
                    "guest wrote more than {} response bytes to stdout",
                    state.max_response_bytes
                ),
            });
        }

        Ok(Bytes::from(std::mem::take(&mut state.response)))
    }
}

impl IsTerminal for AsyncGuestOutputCapture {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for AsyncGuestOutputCapture {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(tokio::io::sink())
    }
}

#[async_trait::async_trait]
impl OutputStream for AsyncGuestOutputCapture {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StreamError::trap("guest async stdout capture lock poisoned"))?;
        state.pending.extend_from_slice(&bytes);
        while let Some(position) = state.pending.iter().position(|byte| *byte == b'\n') {
            let segment = state.pending.drain(..=position).collect::<Vec<_>>();
            handle_async_guest_segment(&mut state, &segment);
        }
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(4096)
    }
}

#[async_trait::async_trait]
impl Pollable for AsyncGuestOutputCapture {
    async fn ready(&mut self) {}
}

fn disconnected_log_sender() -> mpsc::Sender<AsyncLogEntry> {
    let (sender, _receiver) = mpsc::channel(1);
    sender
}

struct GuestTempFile {
    path: PathBuf,
    file: fs::File,
}

impl Drop for GuestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_guest_stdin_file(body: &Bytes) -> std::result::Result<GuestTempFile, ExecutionError> {
    let path = guest_temp_file_path("stdin");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(&path)
        .map_err(|error| {
            guest_execution_error(error.into(), "failed to create guest stdin temp file")
        })?;
    file.write_all(body).map_err(|error| {
        guest_execution_error(error.into(), "failed to write guest stdin temp file")
    })?;
    file.flush().map_err(|error| {
        guest_execution_error(error.into(), "failed to flush guest stdin temp file")
    })?;
    file.sync_all().map_err(|error| {
        guest_execution_error(error.into(), "failed to sync guest stdin temp file to disk")
    })?;
    drop(file);
    let file = fs::File::open(&path).map_err(|error| {
        guest_execution_error(error.into(), "failed to reopen guest stdin temp file")
    })?;
    Ok(GuestTempFile { path, file })
}

#[cfg(test)]
fn create_guest_stdout_file() -> std::result::Result<GuestTempFile, ExecutionError> {
    let path = guest_temp_file_path("stdout");
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(&path)
        .map_err(|error| {
            guest_execution_error(error.into(), "failed to create guest stdout temp file")
        })?;
    Ok(GuestTempFile { path, file })
}

fn guest_temp_file_path(kind: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("tachyon-{kind}-{}.tmp", Uuid::new_v4()));
    path
}

#[cfg(test)]
fn read_guest_stdout_file(
    path: &Path,
    max_stdout_bytes: usize,
) -> std::result::Result<Bytes, ExecutionError> {
    let stdout = fs::read(path).map_err(|error| {
        guest_execution_error(error.into(), "failed to read guest stdout temp file")
    })?;
    if stdout.len() > max_stdout_bytes {
        return Err(ExecutionError::ResourceLimitExceeded {
            kind: ResourceLimitKind::Stdout,
            detail: format!(
                "guest wrote {} bytes to stdout with a configured limit of {} bytes",
                stdout.len(),
                max_stdout_bytes
            ),
        });
    }
    Ok(Bytes::from(stdout))
}

fn maybe_set_guest_fuel_budget<T>(
    store: &mut Store<T>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    if !execution.sampled_execution {
        return Ok(());
    }

    store
        .set_fuel(execution.config.guest_fuel_budget)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))
}

fn sampled_fuel_consumed<T>(
    store: &mut Store<T>,
    execution: &GuestExecutionContext,
) -> std::result::Result<Option<u64>, ExecutionError> {
    if !execution.sampled_execution {
        return Ok(None);
    }

    let remaining = store
        .get_fuel()
        .map_err(|error| guest_execution_error(error, "failed to read remaining guest fuel"))?;
    Ok(Some(
        execution.config.guest_fuel_budget.saturating_sub(remaining),
    ))
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
    #[cfg(feature = "ai-inference")]
    wasmtime_wasi_nn::witx::add_to_linker(&mut linker, |state: &mut LegacyHostState| {
        &mut state.wasi_nn
    })
    .map_err(|error| guest_execution_error(error, "failed to add WASI-NN functions to linker"))?;
    Ok(linker)
}

#[cfg_attr(feature = "ai-inference", allow(dead_code))]
fn requires_ai_inference_feature(function_name: &str) -> bool {
    normalize_target_module_name(function_name) == "guest-ai"
}

fn resolve_function_name(path: &str) -> Option<String> {
    path.split('/')
        .rev()
        .find(|segment| !segment.is_empty() && *segment != "api")
        .map(ToOwned::to_owned)
}

fn default_route_name(path: &str) -> String {
    resolve_function_name(path).unwrap_or_else(|| path.trim_matches('/').to_owned())
}

fn background_route_module(route: &IntegrityRoute) -> Option<String> {
    route
        .targets
        .first()
        .map(|target| target.module.clone())
        .or_else(|| resolve_function_name(&route.path))
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
    let wasm_file = format!(
        "{}.wasm",
        normalize_target_module_name(function_name).replace('-', "_")
    );
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

fn normalize_target_module_name(module_name: &str) -> String {
    module_name
        .trim()
        .strip_suffix(".wasm")
        .unwrap_or(module_name.trim())
        .trim()
        .to_owned()
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

    telemetry::record_event(&telemetry.handle, event);
}

fn should_shed_system_route(telemetry: &TelemetryHandle) -> bool {
    is_system_route_saturated(telemetry::active_requests(telemetry))
}

fn is_system_route_saturated(active_requests: usize) -> bool {
    active_requests > SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD
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

fn build_engine(integrity_config: &IntegrityConfig, enable_fuel_metering: bool) -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(enable_fuel_metering);
    config.wasm_component_model(true);
    config.allocation_strategy(build_pooling_config(integrity_config)?);

    Engine::new(&config)
        .map_err(|error| anyhow!("failed to create Wasmtime engine with pooling enabled: {error}"))
}

fn build_command_engine(_integrity_config: &IntegrityConfig) -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);

    Engine::new(&config)
        .map_err(|error| anyhow!("failed to create Wasmtime engine for batch execution: {error}"))
}

fn build_pooling_config(config: &IntegrityConfig) -> Result<PoolingAllocationConfig> {
    let total_route_concurrency = total_route_concurrency(&config.routes)?;
    let total_min_instances = total_min_instances(&config.routes)?;
    let mut pooling = PoolingAllocationConfig::new();

    pooling.total_component_instances(total_route_concurrency);
    pooling.total_core_instances(
        total_route_concurrency.saturating_mul(POOLING_CORE_INSTANCES_MULTIPLIER),
    );
    pooling.total_memories(total_route_concurrency.saturating_mul(POOLING_MEMORIES_MULTIPLIER));
    pooling.total_tables(total_route_concurrency.saturating_mul(POOLING_TABLES_MULTIPLIER));
    pooling.max_component_instance_size(POOLING_INSTANCE_METADATA_BYTES);
    pooling.max_core_instance_size(POOLING_INSTANCE_METADATA_BYTES);
    pooling.max_core_instances_per_component(POOLING_MAX_CORE_INSTANCES_PER_COMPONENT);
    pooling.max_memories_per_component(POOLING_MAX_MEMORIES_PER_COMPONENT);
    pooling.max_tables_per_component(POOLING_MAX_TABLES_PER_COMPONENT);
    pooling.max_memory_size(config.guest_memory_limit_bytes);
    pooling.max_unused_warm_slots(total_min_instances);

    Ok(pooling)
}

fn build_concurrency_limits(
    config: &IntegrityConfig,
) -> Arc<HashMap<String, Arc<RouteExecutionControl>>> {
    Arc::new(
        config
            .routes
            .iter()
            .map(|route| {
                (
                    route.path.clone(),
                    Arc::new(RouteExecutionControl::new(route)),
                )
            })
            .collect(),
    )
}

fn total_route_concurrency(routes: &[IntegrityRoute]) -> Result<u32> {
    u32::try_from(
        routes
            .iter()
            .map(|route| u64::from(route.max_concurrency))
            .sum::<u64>(),
    )
    .context("embedded sealed configuration declares more route concurrency than Wasmtime can pool")
}

fn total_min_instances(routes: &[IntegrityRoute]) -> Result<u32> {
    u32::try_from(
        routes
            .iter()
            .map(|route| u64::from(route.min_instances))
            .sum::<u64>(),
    )
    .context("embedded sealed configuration declares more warm instances than Wasmtime can track")
}

impl RouteExecutionControl {
    fn new(route: &IntegrityRoute) -> Self {
        Self::from_limits(route.min_instances, route.max_concurrency)
    }

    fn from_limits(min_instances: u32, max_concurrency: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(
                usize::try_from(max_concurrency)
                    .expect("route max_concurrency should fit in usize"),
            )),
            pending_waiters: AtomicUsize::new(0),
            active_requests: AtomicUsize::new(0),
            draining: AtomicBool::new(false),
            draining_since: Mutex::new(None),
            min_instances,
            max_concurrency,
            prewarmed_instances: AtomicUsize::new(0),
        }
    }

    fn pending_queue_size(&self) -> u32 {
        self.pending_waiters
            .load(Ordering::Relaxed)
            .min(u32::MAX as usize) as u32
    }

    fn record_prewarm_success(&self) {
        self.prewarmed_instances.fetch_add(1, Ordering::SeqCst);
    }

    fn begin_request(self: &Arc<Self>) -> ActiveRouteRequestGuard {
        ActiveRouteRequestGuard::new(Arc::clone(self))
    }

    fn mark_draining(&self, started_at: Instant) {
        self.draining.store(true, Ordering::SeqCst);
        *self
            .draining_since
            .lock()
            .expect("route lifecycle state should not be poisoned") = Some(started_at);
    }

    fn force_terminate(&self) {
        self.semaphore.close();
    }

    fn lifecycle_state(&self) -> RouteLifecycleState {
        if self.draining.load(Ordering::SeqCst) {
            RouteLifecycleState::Draining
        } else {
            RouteLifecycleState::Active
        }
    }

    fn active_request_count(&self) -> usize {
        self.active_requests.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn prewarmed_instances(&self) -> u32 {
        self.prewarmed_instances
            .load(Ordering::SeqCst)
            .min(u32::MAX as usize) as u32
    }
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
    let config = verify_integrity_payload(
        EMBEDDED_CONFIG_PAYLOAD,
        EMBEDDED_PUBLIC_KEY,
        EMBEDDED_SIGNATURE,
        "embedded sealed configuration",
    )?;
    tracing::info!("integrity verification passed");
    Ok(config)
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn load_integrity_config_from_manifest_path(path: &Path) -> Result<IntegrityConfig> {
    let manifest = read_integrity_manifest(path)?;
    verify_integrity_payload(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        &format!("integrity manifest at {}", path.display()),
    )
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
fn read_integrity_manifest(path: &Path) -> Result<IntegrityManifest> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read integrity manifest at {}", path.display()))?;

    serde_json::from_str(&manifest)
        .with_context(|| format!("failed to parse integrity manifest at {}", path.display()))
}

fn verify_integrity_payload(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
    source: &str,
) -> Result<IntegrityConfig> {
    verify_integrity_signature(payload, public_key_hex, signature_hex)?;
    let config = serde_json::from_str::<IntegrityConfig>(payload)
        .with_context(|| format!("failed to parse {source}"))?;
    validate_integrity_config(config)
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

    if !config.telemetry_sample_rate.is_finite()
        || !(0.0..=1.0).contains(&config.telemetry_sample_rate)
    {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must set `telemetry_sample_rate` between 0.0 and 1.0"
        ));
    }

    if config.routes.is_empty() && config.batch_targets.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must define at least one route or batch target"
        ));
    }

    config.tls_address = normalize_tls_address(config.tls_address)?;
    config.batch_targets = normalize_batch_targets(config.batch_targets)?;
    config.routes = normalize_config_routes(config.routes, !config.batch_targets.is_empty())?;
    let route_registry = RouteRegistry::build(&config)?;
    config.layer4 = normalize_layer4_config(config.layer4, &route_registry)?;
    Ok(config)
}

fn normalize_config_routes(
    routes: Vec<IntegrityRoute>,
    allow_empty: bool,
) -> Result<Vec<IntegrityRoute>> {
    if routes.is_empty() {
        if allow_empty {
            return Ok(Vec::new());
        }
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must define at least one route"
        ));
    }

    let mut normalized = routes
        .into_iter()
        .map(validate_integrity_route)
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by(|left, right| left.path.cmp(&right.path));
    for pair in normalized.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{}` is defined more than once",
                pair[0].path
            ));
        }
    }
    ensure_unique_model_aliases(&normalized)?;
    ensure_unique_route_domains(&normalized)?;
    Ok(normalized)
}

fn normalize_batch_targets(
    targets: Vec<IntegrityBatchTarget>,
) -> Result<Vec<IntegrityBatchTarget>> {
    let mut normalized = targets
        .into_iter()
        .map(validate_integrity_batch_target)
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in normalized.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(anyhow!(
                "Integrity Validation Failed: batch target `{}` is defined more than once",
                pair[0].name
            ));
        }
    }

    Ok(normalized)
}

fn normalize_layer4_config(
    mut layer4: IntegrityLayer4Config,
    route_registry: &RouteRegistry,
) -> Result<IntegrityLayer4Config> {
    layer4.tcp = normalize_tcp_bindings(layer4.tcp, route_registry)?;
    layer4.udp = normalize_udp_bindings(layer4.udp, route_registry)?;
    Ok(layer4)
}

fn normalize_tcp_bindings(
    bindings: Vec<IntegrityTcpBinding>,
    route_registry: &RouteRegistry,
) -> Result<Vec<IntegrityTcpBinding>> {
    let mut normalized = bindings
        .into_iter()
        .map(|binding| {
            if binding.port == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: TCP Layer 4 bindings must use a port above zero"
                ));
            }

            let target = normalize_service_name(&binding.target).map_err(|error| {
                anyhow!("Integrity Validation Failed: TCP Layer 4 target is invalid: {error}")
            })?;
            route_registry
                .resolve_named_route(&target)
                .map_err(|error| {
                    anyhow!(
                    "Integrity Validation Failed: TCP Layer 4 target `{target}` is invalid: {error}"
                )
                })?;

            Ok(IntegrityTcpBinding {
                port: binding.port,
                target,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by_key(|binding| binding.port);
    for pair in normalized.windows(2) {
        if pair[0].port == pair[1].port {
            return Err(anyhow!(
                "Integrity Validation Failed: TCP Layer 4 port `{}` is defined more than once",
                pair[0].port
            ));
        }
    }

    Ok(normalized)
}

fn normalize_udp_bindings(
    bindings: Vec<IntegrityUdpBinding>,
    route_registry: &RouteRegistry,
) -> Result<Vec<IntegrityUdpBinding>> {
    let mut normalized = bindings
        .into_iter()
        .map(|binding| {
            if binding.port == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: UDP Layer 4 bindings must use a port above zero"
                ));
            }

            let target = normalize_service_name(&binding.target).map_err(|error| {
                anyhow!("Integrity Validation Failed: UDP Layer 4 target is invalid: {error}")
            })?;
            route_registry
                .resolve_named_route(&target)
                .map_err(|error| {
                    anyhow!(
                        "Integrity Validation Failed: UDP Layer 4 target `{target}` is invalid: {error}"
                    )
                })?;

            Ok(IntegrityUdpBinding {
                port: binding.port,
                target,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by_key(|binding| binding.port);
    for pair in normalized.windows(2) {
        if pair[0].port == pair[1].port {
            return Err(anyhow!(
                "Integrity Validation Failed: UDP Layer 4 port `{}` is defined more than once",
                pair[0].port
            ));
        }
    }

    Ok(normalized)
}

fn validate_integrity_route(route: IntegrityRoute) -> Result<IntegrityRoute> {
    let normalized = normalize_route_path(&route.path);

    if normalized == "/" || resolve_function_name(&normalized).is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` does not resolve to a guest function"
        ));
    }

    if route.max_concurrency == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` must set `max_concurrency` above zero"
        ));
    }

    if route.min_instances > route.max_concurrency {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` cannot set `min_instances` above `max_concurrency`"
        ));
    }

    Ok(IntegrityRoute {
        path: normalized.clone(),
        role: route.role,
        name: normalize_route_name(&route.name, &normalized)?,
        version: normalize_route_version(&route.version, &normalized)?,
        dependencies: normalize_route_dependencies(route.dependencies, &normalized)?,
        requires_credentials: normalize_route_credentials(route.requires_credentials)?,
        middleware: normalize_route_middleware(route.middleware, &normalized)?,
        env: normalize_route_env(route.env, &normalized)?,
        allowed_secrets: normalize_allowed_secrets(route.allowed_secrets)?,
        targets: normalize_route_targets(route.targets)?,
        resiliency: normalize_route_resiliency(route.resiliency, &normalized)?,
        models: normalize_route_models(route.models, &normalized)?,
        domains: normalize_route_domains(route.domains, &normalized)?,
        min_instances: route.min_instances,
        max_concurrency: route.max_concurrency,
        volumes: normalize_route_volumes(route.volumes, route.role, &normalized)?,
    })
}

fn validate_integrity_batch_target(target: IntegrityBatchTarget) -> Result<IntegrityBatchTarget> {
    let name = target.name.trim();
    if name.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: batch targets must include a non-empty `name`"
        ));
    }

    let module = target.module.trim();
    if module.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: batch target `{name}` must include a non-empty `module`"
        ));
    }

    let env = target
        .env
        .into_iter()
        .map(|(key, value)| {
            let normalized_key = key.trim().to_owned();
            if normalized_key.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: batch target `{name}` environment keys cannot be empty"
                ));
            }

            Ok((normalized_key, value.trim().to_owned()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    Ok(IntegrityBatchTarget {
        name: name.to_owned(),
        module: module.to_owned(),
        env,
        volumes: normalize_route_volumes(
            target.volumes,
            RouteRole::System,
            &format!("batch target `{name}`"),
        )?,
    })
}

fn normalize_route_targets(targets: Vec<RouteTarget>) -> Result<Vec<RouteTarget>> {
    targets
        .into_iter()
        .map(normalize_route_target)
        .collect::<Result<Vec<_>>>()
}

fn normalize_route_resiliency(
    resiliency: Option<ResiliencyConfig>,
    route_path: &str,
) -> Result<Option<ResiliencyConfig>> {
    let Some(resiliency) = resiliency else {
        return Ok(None);
    };

    let timeout_ms = resiliency
        .timeout_ms
        .map(|timeout_ms| {
            if timeout_ms == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` must set `resiliency.timeout_ms` above zero"
                ));
            }
            Ok(timeout_ms)
        })
        .transpose()?;

    let retry_policy = resiliency
        .retry_policy
        .map(|policy| normalize_retry_policy(policy, route_path))
        .transpose()?;

    if timeout_ms.is_none() && retry_policy.is_none() {
        return Ok(None);
    }

    Ok(Some(ResiliencyConfig {
        timeout_ms,
        retry_policy,
    }))
}

fn normalize_retry_policy(policy: RetryPolicy, route_path: &str) -> Result<RetryPolicy> {
    let retry_on = policy
        .retry_on
        .into_iter()
        .map(|status| {
            if !(100..=599).contains(&status) {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` has an invalid `resiliency.retry_policy.retry_on` status `{status}`"
                ));
            }
            Ok(status)
        })
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect::<Vec<_>>();

    if policy.max_retries == 0 && !retry_on.is_empty() {
        return Ok(RetryPolicy {
            max_retries: 0,
            retry_on,
        });
    }

    if policy.max_retries > 0 && retry_on.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` must configure at least one `resiliency.retry_policy.retry_on` status when `max_retries` is set"
        ));
    }

    Ok(RetryPolicy {
        max_retries: policy.max_retries,
        retry_on,
    })
}

fn normalize_route_models(
    models: Vec<IntegrityModelBinding>,
    route_path: &str,
) -> Result<Vec<IntegrityModelBinding>> {
    let mut deduped = BTreeMap::new();

    for model in models {
        let alias = normalize_service_name(&model.alias).map_err(|error| {
            anyhow!(
                "Integrity Validation Failed: route `{route_path}` has an invalid model alias `{}`: {error}",
                model.alias
            )
        })?;
        let path = model.path.trim();
        if path.is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{route_path}` model `{alias}` must include a non-empty `path`"
            ));
        }

        deduped
            .entry(alias.clone())
            .or_insert(IntegrityModelBinding {
                alias,
                path: path.to_owned(),
                device: model.device,
                qos: model.qos,
            });
    }

    Ok(deduped.into_values().collect())
}

fn normalize_route_domains(domains: Vec<String>, route_path: &str) -> Result<Vec<String>> {
    domains
        .into_iter()
        .map(|domain| {
            tls_runtime::normalize_domain(&domain).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{route_path}` has an invalid domain `{domain}`: {error}"
                )
            })
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|domains| domains.into_iter().collect())
}

fn ensure_unique_route_domains(routes: &[IntegrityRoute]) -> Result<()> {
    let mut owners = HashMap::new();

    for route in routes {
        for domain in &route.domains {
            if let Some(previous_route) = owners.insert(domain.clone(), route.path.clone()) {
                return Err(anyhow!(
                    "Integrity Validation Failed: domain `{domain}` is declared by both route `{previous_route}` and route `{}`",
                    route.path
                ));
            }
        }
    }

    Ok(())
}

fn ensure_unique_model_aliases(routes: &[IntegrityRoute]) -> Result<()> {
    let mut owners = HashMap::new();

    for route in routes {
        for model in &route.models {
            if let Some(previous_route) = owners.insert(model.alias.clone(), route.path.clone()) {
                return Err(anyhow!(
                    "Integrity Validation Failed: model alias `{}` is declared by both route `{previous_route}` and route `{}`",
                    model.alias, route.path
                ));
            }
        }
    }

    Ok(())
}

fn normalize_tls_address(address: Option<String>) -> Result<Option<String>> {
    address
        .map(|address| {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: `tls_address` must not be empty"
                ));
            }
            trimmed.parse::<SocketAddr>().map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: `tls_address` must be a socket address: {error}"
                )
            })?;
            Ok(trimmed.to_owned())
        })
        .transpose()
}

fn normalize_route_name(name: &str, path: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(default_route_name(path));
    }
    normalize_service_name(trimmed).map_err(|error| {
        anyhow!("Integrity Validation Failed: route `{path}` has an invalid `name`: {error}")
    })
}

fn normalize_service_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("service names cannot be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(anyhow!("service names must not contain path separators"));
    }

    Ok(trimmed.to_owned())
}

fn normalize_route_version(version: &str, path: &str) -> Result<String> {
    Version::parse(version.trim())
        .with_context(|| {
            format!(
                "Integrity Validation Failed: route `{path}` must use a valid semantic `version`"
            )
        })
        .map(|parsed| parsed.to_string())
}

fn normalize_route_dependencies(
    dependencies: BTreeMap<String, String>,
    path: &str,
) -> Result<BTreeMap<String, String>> {
    dependencies
        .into_iter()
        .map(|(name, requirement)| {
            let normalized_name = normalize_service_name(&name).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{path}` has an invalid dependency name `{}`: {}",
                    name,
                    error
                )
            })?;
            let parsed = VersionReq::parse(requirement.trim()).with_context(|| {
                format!(
                    "Integrity Validation Failed: route `{path}` has an invalid dependency requirement for `{normalized_name}`"
                )
            })?;
            Ok((normalized_name, parsed.to_string()))
        })
        .collect()
}

fn normalize_route_credentials(credentials: Vec<String>) -> Result<Vec<String>> {
    credentials
        .into_iter()
        .map(|credential| {
            let trimmed = credential.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route credentials must not be empty"
                ));
            }
            Ok(trimmed.to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|credentials| credentials.into_iter().collect())
}

fn normalize_route_middleware(middleware: Option<String>, path: &str) -> Result<Option<String>> {
    middleware
        .map(|middleware| {
            normalize_service_name(&middleware).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{path}` has an invalid `middleware`: {error}"
                )
            })
        })
        .transpose()
}

fn normalize_route_env(
    env: BTreeMap<String, String>,
    route_path: &str,
) -> Result<BTreeMap<String, String>> {
    env.into_iter()
        .map(|(key, value)| {
            let normalized_key = key.trim().to_owned();
            if normalized_key.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` environment keys cannot be empty"
                ));
            }
            Ok((normalized_key, value.trim().to_owned()))
        })
        .collect()
}

fn normalize_route_target(target: RouteTarget) -> Result<RouteTarget> {
    let module = normalize_target_module_name(&target.module);
    if module.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route targets must include a non-empty `module`"
        ));
    }
    if module.contains('/') || module.contains('\\') {
        return Err(anyhow!(
            "Integrity Validation Failed: route targets must use module names, not filesystem paths"
        ));
    }
    if target.weight > 100 {
        return Err(anyhow!(
            "Integrity Validation Failed: route target `{module}` must keep `weight` between 0 and 100"
        ));
    }

    Ok(RouteTarget {
        module,
        weight: target.weight,
        websocket: target.websocket,
        match_header: target
            .match_header
            .map(normalize_header_match)
            .transpose()?,
    })
}

fn normalize_header_match(header_match: HeaderMatch) -> Result<HeaderMatch> {
    let name = header_match.name.trim().to_ascii_lowercase();
    let value = header_match.value.trim().to_owned();

    if name.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route target header matches must include a non-empty `name`"
        ));
    }
    if value.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route target header matches must include a non-empty `value`"
        ));
    }

    Ok(HeaderMatch { name, value })
}

fn normalize_route_volumes(
    volumes: Vec<IntegrityVolume>,
    route_role: RouteRole,
    route_path: &str,
) -> Result<Vec<IntegrityVolume>> {
    let mut normalized = BTreeSet::new();
    let mut deduped = Vec::new();

    for volume in volumes {
        let volume = validate_route_volume(volume, route_role, route_path)?;
        if !normalized.insert((
            volume.volume_type.clone(),
            volume.guest_path.clone(),
            volume.host_path.clone(),
            volume.readonly,
            volume.ttl_seconds,
            volume.idle_timeout.clone(),
            volume.eviction_policy.clone(),
        )) {
            continue;
        }

        if deduped
            .iter()
            .any(|existing: &IntegrityVolume| existing.guest_path == volume.guest_path)
        {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{route_path}` defines guest volume path `{}` more than once",
                volume.guest_path
            ));
        }

        deduped.push(volume);
    }

    deduped.sort_by(|left, right| left.guest_path.cmp(&right.guest_path));
    Ok(deduped)
}

fn validate_route_volume(
    volume: IntegrityVolume,
    route_role: RouteRole,
    route_path: &str,
) -> Result<IntegrityVolume> {
    let host_path = volume.host_path.trim();
    if host_path.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route volumes must include a non-empty `host_path`"
        ));
    }

    if route_role == RouteRole::User && volume.volume_type == VolumeType::Host && !volume.readonly {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` is a user route and cannot request writable direct host mounts; use a system storage broker instead"
        ));
    }

    let guest_path = normalize_guest_volume_path(&volume.guest_path)?;
    if volume.ttl_seconds.is_some_and(|ttl| ttl == 0) {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` must set `ttl_seconds` above zero"
        ));
    }
    let idle_timeout = volume
        .idle_timeout
        .as_deref()
        .map(|timeout| normalize_idle_timeout(timeout, route_path, &guest_path))
        .transpose()?;

    if volume.eviction_policy.is_some() && volume.volume_type != VolumeType::Ram {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` can only set `eviction_policy` for `type = \"ram\"` volumes"
        ));
    }

    if idle_timeout.is_some() && volume.volume_type != VolumeType::Ram {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` can only set `idle_timeout` for `type = \"ram\"` volumes"
        ));
    }

    if volume.eviction_policy == Some(VolumeEvictionPolicy::Hibernate) && idle_timeout.is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` must set `idle_timeout` when `eviction_policy = \"hibernate\"`"
        ));
    }

    Ok(IntegrityVolume {
        volume_type: volume.volume_type,
        host_path: host_path.to_owned(),
        guest_path,
        readonly: volume.readonly,
        ttl_seconds: volume.ttl_seconds,
        idle_timeout,
        eviction_policy: volume.eviction_policy,
    })
}

fn normalize_guest_volume_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route volumes must include a non-empty `guest_path`"
        ));
    }
    if !trimmed.starts_with('/') {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths must be absolute, for example `/app/data`"
        ));
    }
    if trimmed.contains('\\') {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths must use `/` separators"
        ));
    }

    let normalized = trimmed.trim_end_matches('/');
    let normalized = if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized.to_owned()
    };

    if normalized == "/" {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume path `/` is not allowed"
        ));
    }
    if normalized
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths cannot contain empty, `.` or `..` segments"
        ));
    }

    Ok(normalized)
}

fn normalize_idle_timeout(value: &str, route_path: &str, guest_path: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` has an empty `idle_timeout`"
        ));
    }

    parse_hibernation_duration(trimmed).with_context(|| {
        format!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` has an invalid `idle_timeout`"
        )
    })?;

    Ok(trimmed.to_owned())
}

fn parse_hibernation_duration(value: &str) -> Result<Duration> {
    let trimmed = value.trim();
    let (digits, multiplier) = if let Some(value) = trimmed.strip_suffix("ms") {
        (value, Duration::from_millis(1))
    } else if let Some(value) = trimmed.strip_suffix('s') {
        (value, Duration::from_secs(1))
    } else if let Some(value) = trimmed.strip_suffix('m') {
        (value, Duration::from_secs(60))
    } else {
        return Err(anyhow!(
            "idle_timeout must use one of the `ms`, `s`, or `m` suffixes"
        ));
    };

    let amount = digits
        .trim()
        .parse::<u64>()
        .context("idle_timeout must start with an unsigned integer")?;
    if amount == 0 {
        return Err(anyhow!("idle_timeout must be greater than zero"));
    }

    multiplier
        .checked_mul(u32::try_from(amount).context("idle_timeout is too large")?)
        .ok_or_else(|| anyhow!("idle_timeout is too large"))
}

fn normalize_allowed_secrets(allowed_secrets: Vec<String>) -> Result<Vec<String>> {
    allowed_secrets
        .into_iter()
        .map(|secret| {
            let trimmed = secret.trim();
            if trimmed.is_empty() {
                Err(anyhow!(
                    "Integrity Validation Failed: allowed secret names cannot be empty"
                ))
            } else {
                Ok(trimmed.to_owned())
            }
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|allowed_secrets| allowed_secrets.into_iter().collect())
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

#[cfg(test)]
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

fn flush_async_guest_output(state: &mut AsyncGuestOutputState) {
    if state.pending.is_empty() {
        return;
    }

    let segment = std::mem::take(&mut state.pending);
    handle_async_guest_segment(state, &segment);
}

fn handle_async_guest_segment(state: &mut AsyncGuestOutputState, segment: &[u8]) {
    let text = String::from_utf8_lossy(segment);
    let line = trim_line_endings(&text);
    if line.is_empty() {
        if state.capture_response {
            append_async_guest_response(state, segment);
        }
        return;
    }

    if let Some(record) = parse_guest_log_line(line) {
        enqueue_structured_guest_log(state, record);
        return;
    }

    if state.capture_response {
        append_async_guest_response(state, segment);
    } else {
        enqueue_raw_guest_log(state, line.to_owned());
    }
}

fn append_async_guest_response(state: &mut AsyncGuestOutputState, segment: &[u8]) {
    if state.response_overflowed {
        return;
    }

    state.response.extend_from_slice(segment);
    if state.response.len() > state.max_response_bytes {
        state.response_overflowed = true;
    }
}

fn enqueue_structured_guest_log(state: &AsyncGuestOutputState, record: GuestLogRecord) {
    let message = record
        .fields
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("guest emitted a structured log")
        .to_owned();
    enqueue_async_guest_log(
        state,
        record.level,
        message,
        record.target,
        Some(Value::Object(record.fields)),
    );
}

fn enqueue_raw_guest_log(state: &AsyncGuestOutputState, message: String) {
    let level = match state.stream_type {
        Some(GuestLogStreamType::Stderr) => "error",
        _ => "info",
    };
    enqueue_async_guest_log(state, level.to_owned(), message, None, None);
}

fn enqueue_async_guest_log(
    state: &AsyncGuestOutputState,
    level: String,
    message: String,
    guest_target: Option<String>,
    structured_fields: Option<Value>,
) {
    let Some(sender) = &state.sender else {
        return;
    };
    let Some(stream_type) = state.stream_type else {
        return;
    };
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default();

    let _ = sender.try_send(AsyncLogEntry {
        target_name: state.function_name.clone(),
        timestamp_unix_ms,
        stream_type,
        level,
        message,
        guest_target,
        structured_fields,
    });
}

#[cfg(test)]
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
    fn new(
        wasi: WasiP1Ctx,
        max_memory_bytes: usize,
        #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
    ) -> Self {
        Self {
            wasi,
            #[cfg(feature = "ai-inference")]
            wasi_nn: build_wasi_nn_ctx(ai_runtime.as_ref()),
            limits: GuestResourceLimiter::new(max_memory_bytes),
        }
    }
}

#[cfg(feature = "ai-inference")]
fn build_wasi_nn_ctx(runtime: &ai_inference::AiInferenceRuntime) -> WasiNnCtx {
    runtime.build_wasi_nn_ctx()
}

impl SecretsVault {
    fn load() -> Self {
        #[cfg(feature = "secrets-vault")]
        {
            let entries = HashMap::from([("DB_PASS".to_owned(), "super_secret_123".to_owned())]);
            Self {
                entries: Arc::new(entries),
            }
        }

        #[cfg(not(feature = "secrets-vault"))]
        {
            Self::default()
        }
    }
}

impl SecretAccess {
    fn from_route(route: &IntegrityRoute, _vault: &SecretsVault) -> Self {
        Self {
            allowed_secrets: route.allowed_secrets.iter().cloned().collect(),
            #[cfg(feature = "secrets-vault")]
            entries: Arc::clone(&_vault.entries),
        }
    }

    fn get_secret(&self, name: &str) -> std::result::Result<String, SecretAccessErrorKind> {
        #[cfg(not(feature = "secrets-vault"))]
        {
            let _ = name;
            Err(SecretAccessErrorKind::VaultDisabled)
        }

        #[cfg(feature = "secrets-vault")]
        {
            if !self.allowed_secrets.contains(name) {
                return Err(SecretAccessErrorKind::PermissionDenied);
            }

            self.entries
                .get(name)
                .cloned()
                .ok_or(SecretAccessErrorKind::NotFound)
        }
    }
}

impl ComponentHostState {
    #[allow(clippy::too_many_arguments)]
    fn new(
        route: &IntegrityRoute,
        runtime_config: IntegrityConfig,
        max_memory_bytes: usize,
        telemetry: TelemetryHandle,
        secrets: SecretAccess,
        request_headers: HeaderMap,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
        propagated_headers: Vec<PropagatedHeader>,
    ) -> std::result::Result<Self, ExecutionError> {
        Ok(Self {
            ctx: build_component_wasi_ctx(route, host_identity.as_ref())?,
            table: ResourceTable::new(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
            secrets,
            runtime_config,
            request_headers,
            host_identity,
            storage_broker,
            telemetry,
            concurrency_limits,
            propagated_headers,
            outbound_http_client: reqwest::blocking::Client::new(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: None,
            #[cfg(feature = "ai-inference")]
            allowed_model_aliases: route
                .models
                .iter()
                .map(|binding| binding.alias.clone())
                .collect(),
            #[cfg(feature = "ai-inference")]
            accelerator_models: HashMap::new(),
            #[cfg(feature = "ai-inference")]
            next_accelerator_model_id: 1,
        })
    }

    fn pending_queue_size(&self, route_path: &str) -> u32 {
        self.concurrency_limits
            .get(&normalize_route_path(route_path))
            .map(|control| control.pending_queue_size())
            .unwrap_or_default()
    }

    #[cfg(feature = "ai-inference")]
    fn load_accelerator_model(
        &mut self,
        accelerator: ai_inference::AcceleratorKind,
        alias: String,
    ) -> std::result::Result<u32, String> {
        if !self.allowed_model_aliases.contains(&alias) {
            return Err(format!(
                "model alias `{alias}` is not sealed for this route"
            ));
        }
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| "AI inference runtime is unavailable for this component".to_owned())?
            .load_component_model(&alias, accelerator)?;
        let model_id = self.next_accelerator_model_id;
        self.next_accelerator_model_id = self.next_accelerator_model_id.saturating_add(1);
        self.accelerator_models
            .insert(model_id, LoadedAcceleratorModel { alias, accelerator });
        Ok(model_id)
    }

    #[cfg(feature = "ai-inference")]
    fn compute_accelerator_prompt(
        &self,
        expected_accelerator: ai_inference::AcceleratorKind,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<String, String> {
        let loaded = self
            .accelerator_models
            .get(&model_id)
            .ok_or_else(|| format!("accelerator model handle `{model_id}` is unknown"))?;
        if loaded.accelerator != expected_accelerator {
            return Err(format!(
                "accelerator model handle `{model_id}` was loaded for `{}` not `{}`",
                loaded.accelerator.as_str(),
                expected_accelerator.as_str()
            ));
        }
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| "AI inference runtime is unavailable for this component".to_owned())?
            .compute_component_prompt(&loaded.alias, &prompt)
    }
}

fn build_component_wasi_ctx(
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
) -> std::result::Result<WasiCtx, ExecutionError> {
    // Intentionally do not inherit the host environment. Secrets stay in host memory
    // and are only reachable through the typed vault import.
    let mut wasi = WasiCtxBuilder::new();
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    preopen_route_volumes(&mut wasi, route)?;
    Ok(wasi.build())
}

fn add_route_environment(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
) -> std::result::Result<(), ExecutionError> {
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    Ok(())
}

fn system_runtime_environment(
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
) -> Vec<(String, String)> {
    let mut env = route
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    if route.role == RouteRole::System {
        env.push((
            TACHYON_SYSTEM_PUBLIC_KEY_ENV.to_owned(),
            host_identity.public_key_hex.clone(),
        ));
    }
    env
}

fn preopen_route_volumes(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in &route.volumes {
        if volume.volume_type == VolumeType::Ram {
            fs::create_dir_all(&volume.host_path).map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to initialize RAM volume `{}` for route `{}`: {error}",
                    volume.host_path, route.path
                ))
            })?;
        }
        wasi.preopened_dir(
            &volume.host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to preopen volume `{}` for route `{}` at guest path `{}`: {error}",
                volume.host_path, route.path, volume.guest_path
            ))
        })?;
    }

    Ok(())
}

fn preopen_batch_target_volumes(
    wasi: &mut WasiCtxBuilder,
    target: &IntegrityBatchTarget,
) -> Result<()> {
    for volume in &target.volumes {
        if volume.volume_type == VolumeType::Ram {
            fs::create_dir_all(&volume.host_path).with_context(|| {
                format!(
                    "failed to initialize RAM volume `{}` for batch target `{}`",
                    volume.host_path, target.name
                )
            })?;
        }

        wasi.preopened_dir(
            &volume.host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            anyhow!(
                "failed to preopen volume `{}` for batch target `{}` at guest path `{}`: {error}",
                volume.host_path,
                target.name,
                volume.guest_path
            )
        })?;
    }

    Ok(())
}

fn volume_dir_perms(readonly: bool) -> DirPerms {
    if readonly {
        DirPerms::READ
    } else {
        DirPerms::READ | DirPerms::MUTATE
    }
}

fn volume_file_perms(readonly: bool) -> FilePerms {
    if readonly {
        FilePerms::READ
    } else {
        FilePerms::READ | FilePerms::WRITE
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

impl WasiView for BatchCommandState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for ComponentHostState {
    type Data<'a> = &'a mut Self;
}

impl component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<String, component_bindings::tachyon::mesh::secrets_vault::Error> {
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

impl udp_component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<String, udp_component_bindings::tachyon::mesh::secrets_vault::Error>
    {
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<
        String,
        websocket_component_bindings::tachyon::mesh::secrets_vault::Error,
    > {
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::cpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Cpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Cpu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::gpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Gpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Gpu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::npu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Npu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Npu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::tpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Tpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Tpu, model_id, prompt)
    }
}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::websocket::Host for ComponentHostState {}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::websocket::HostConnection for ComponentHostState {
    fn send(
        &mut self,
        self_: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
        frame: websocket_component_bindings::tachyon::mesh::websocket::Frame,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<HostWebSocketConnection>::new_borrow(self_.rep());
        let connection = self
            .table
            .get(&handle)
            .map_err(|error| format!("failed to access WebSocket connection resource: {error}"))?;
        connection
            .outgoing
            .send(websocket_binding_frame_to_host_frame(frame))
            .map_err(|_| "WebSocket connection is closed".to_owned())
    }

    fn receive(
        &mut self,
        self_: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
    ) -> Option<websocket_component_bindings::tachyon::mesh::websocket::Frame> {
        let handle =
            wasmtime::component::Resource::<HostWebSocketConnection>::new_borrow(self_.rep());
        let connection = match self.table.get_mut(&handle) {
            Ok(connection) => connection,
            Err(_) => return None,
        };
        connection
            .incoming
            .recv()
            .ok()
            .map(host_frame_to_websocket_binding_frame)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
    ) -> wasmtime::Result<()> {
        self.table
            .delete(wasmtime::component::Resource::<HostWebSocketConnection>::new_own(rep.rep()))?;
        Ok(())
    }
}

impl system_component_bindings::tachyon::mesh::telemetry_reader::Host for ComponentHostState {
    fn get_metrics(
        &mut self,
    ) -> system_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let TelemetrySnapshot {
            total_requests,
            completed_requests,
            error_requests,
            active_requests,
            dropped_events,
            last_status,
            total_duration_us,
            total_wasm_duration_us,
            total_host_overhead_us,
        } = telemetry::snapshot(&self.telemetry);

        system_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests,
            completed_requests,
            error_requests,
            active_requests,
            dropped_events,
            last_status,
            total_duration_us,
            total_wasm_duration_us,
            total_host_overhead_us,
        }
    }
}

impl system_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl system_component_bindings::tachyon::mesh::storage_broker::Host for ComponentHostState {
    fn enqueue_write(
        &mut self,
        path: String,
        mode: system_component_bindings::tachyon::mesh::storage_broker::WriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let mode = match mode {
            system_component_bindings::tachyon::mesh::storage_broker::WriteMode::Overwrite => {
                StorageWriteMode::Overwrite
            }
            system_component_bindings::tachyon::mesh::storage_broker::WriteMode::Append => {
                StorageWriteMode::Append
            }
        };
        let (route, resolved) = authorize_storage_broker_write(
            &self.runtime_config,
            &self.request_headers,
            self.host_identity.as_ref(),
            &path,
        )?;

        self.storage_broker
            .enqueue_write_target(route.path, resolved, mode, body)
    }

    fn snapshot_volume(
        &mut self,
        volume_id: String,
        source_path: String,
        snapshot_path: String,
    ) -> std::result::Result<(), String> {
        let source_path = parse_storage_broker_host_path(&source_path, "source_path")?;
        let snapshot_path = parse_storage_broker_host_path(&snapshot_path, "snapshot_path")?;
        drop(self.storage_broker.enqueue_snapshot(
            volume_id,
            &source_path,
            &source_path,
            &snapshot_path,
        )?);
        Ok(())
    }

    fn restore_volume(
        &mut self,
        volume_id: String,
        snapshot_path: String,
        destination_path: String,
    ) -> std::result::Result<(), String> {
        let snapshot_path = parse_storage_broker_host_path(&snapshot_path, "snapshot_path")?;
        let destination_path =
            parse_storage_broker_host_path(&destination_path, "destination_path")?;
        drop(self.storage_broker.enqueue_restore(
            volume_id,
            &destination_path,
            &snapshot_path,
            &destination_path,
        )?);
        Ok(())
    }
}

impl background_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl background_component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<
        background_component_bindings::tachyon::mesh::outbound_http::Response,
        String,
    > {
        let method = reqwest::Method::from_bytes(method.trim().as_bytes())
            .map_err(|error| format!("invalid outbound HTTP method `{method}`: {error}"))?;
        let url = rewrite_outbound_http_url(&url, &self.runtime_config);

        tracing::info!(
            method = %method,
            url = %url,
            bytes = body.len(),
            "autoscaling guest sending outbound HTTP request"
        );

        let mut request = self.outbound_http_client.request(method, &url);
        for (name, value) in headers {
            request = request.header(&name, &value);
        }
        for header in &self.propagated_headers {
            request = request.header(&header.name, &header.value);
        }
        let response = request
            .body(body)
            .send()
            .map_err(|error| format!("failed to send outbound HTTP request to `{url}`: {error}"))?;
        let status = response.status().as_u16();
        let response_headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let body = response
            .bytes()
            .map_err(|error| {
                format!("failed to read outbound HTTP response body from `{url}`: {error}")
            })?
            .to_vec();

        Ok(
            background_component_bindings::tachyon::mesh::outbound_http::Response {
                status,
                headers: response_headers,
                body,
            },
        )
    }
}

fn rewrite_outbound_http_url(url: &str, runtime_config: &IntegrityConfig) -> String {
    if let Some(path) = url.strip_prefix("http://mesh") {
        let host = runtime_config
            .host_address
            .parse::<SocketAddr>()
            .map(|address| SocketAddr::new(loopback_ip_for(address.ip()), address.port()))
            .map(|address| address.to_string())
            .unwrap_or_else(|_| runtime_config.host_address.clone());
        return format!("http://{host}{path}");
    }

    let Some(mock_base_url) = std::env::var(MOCK_K8S_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
    else {
        return url.to_owned();
    };

    if let Some(suffix) = url.strip_prefix(KUBERNETES_SERVICE_BASE_URL) {
        format!("{mock_base_url}{suffix}")
    } else {
        url.to_owned()
    }
}

fn loopback_ip_for(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    }
}

impl IntegrityConfig {
    #[cfg(test)]
    fn default_sealed() -> Self {
        Self {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            routes: vec![
                IntegrityRoute::user_with_secrets(DEFAULT_ROUTE, &["DB_PASS"]),
                IntegrityRoute::system(DEFAULT_SYSTEM_ROUTE),
            ],
        }
    }

    fn sealed_route(&self, path: &str) -> Option<&IntegrityRoute> {
        let normalized = normalize_route_path(path);
        self.routes.iter().find(|route| route.path == normalized)
    }

    fn route_for_domain(&self, domain: &str) -> Option<&IntegrityRoute> {
        let normalized = tls_runtime::normalize_domain(domain).ok()?;
        self.routes.iter().find(|route| {
            route
                .domains
                .iter()
                .any(|candidate| candidate == &normalized)
        })
    }

    fn has_custom_domains(&self) -> bool {
        self.routes.iter().any(|route| !route.domains.is_empty())
    }
}

impl IntegrityRoute {
    #[cfg(test)]
    fn user(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            role: RouteRole::User,
            name: default_route_name(path),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: Vec::new(),
        }
    }

    #[cfg(test)]
    fn system(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            role: RouteRole::System,
            name: default_route_name(path),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: Vec::new(),
        }
    }

    #[cfg(test)]
    fn user_with_secrets(path: &str, allowed_secrets: &[&str]) -> Self {
        Self {
            path: path.to_owned(),
            role: RouteRole::User,
            name: default_route_name(path),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: allowed_secrets
                .iter()
                .map(|secret| (*secret).to_owned())
                .collect(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: Vec::new(),
        }
    }
}

impl IntegrityVolume {
    fn is_hibernation_capable(&self) -> bool {
        self.volume_type == VolumeType::Ram
            && self.eviction_policy == Some(VolumeEvictionPolicy::Hibernate)
            && self.idle_timeout.is_some()
    }

    fn parsed_idle_timeout(&self) -> Result<Option<Duration>> {
        self.idle_timeout
            .as_deref()
            .map(parse_hibernation_duration)
            .transpose()
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
            Self::Stdout => f.write_str("stdout"),
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

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GuestModuleNotFound(error) => write!(f, "{error}"),
            Self::ResourceLimitExceeded { kind, detail } => {
                write!(f, "guest exceeded its {kind} quota: {detail}")
            }
            Self::Internal(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for ExecutionError {}

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
        match self {
            Self::ResourceLimitExceeded { kind, detail } => {
                eprintln!("warning: guest `{function_name}` exceeded its {kind} quota: {detail}");
            }
            Self::Internal(message) => {
                eprintln!("error: guest `{function_name}` failed: {message}");
            }
            Self::GuestModuleNotFound(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use ed25519_dalek::{Signer, SigningKey};
    use http_body_util::{BodyExt, Full};
    use prost::Message;
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::{
        rustls::{
            self,
            client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
            pki_types::{CertificateDer, ServerName, UnixTime},
            DigitallySignedStruct, Error as RustlsError, SignatureScheme,
        },
        TlsConnector,
    };
    use tower::util::ServiceExt;

    type CapturedForwardedHeaders = Arc<std::sync::Mutex<Vec<(String, String, String, String)>>>;

    #[derive(Clone, PartialEq, Message)]
    struct TestGrpcHelloRequest {
        #[prost(string, tag = "1")]
        name: String,
    }

    #[derive(Clone, PartialEq, Message)]
    struct TestGrpcHelloResponse {
        #[prost(string, tag = "1")]
        message: String,
    }

    fn expected_secret_status() -> &'static str {
        if cfg!(feature = "secrets-vault") {
            "super_secret_123"
        } else {
            "vault-disabled"
        }
    }

    fn expected_guest_example_body(payload: &str) -> String {
        format!(
            "{payload} | env: missing | secret: {}",
            expected_secret_status()
        )
    }

    fn encode_test_grpc_message<T>(message: &T) -> Vec<u8>
    where
        T: Message,
    {
        let mut payload = Vec::new();
        message
            .encode(&mut payload)
            .expect("protobuf payload should encode");

        let mut framed = Vec::with_capacity(payload.len() + 5);
        framed.push(0);
        framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        framed.extend_from_slice(&payload);
        framed
    }

    fn decode_test_grpc_message<T>(payload: &[u8]) -> T
    where
        T: Message + Default,
    {
        assert!(
            payload.len() >= 5,
            "gRPC payload should include a frame header"
        );
        assert_eq!(payload[0], 0, "test gRPC payload should not be compressed");
        let message_len =
            u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]) as usize;
        let framed = &payload[5..];
        assert_eq!(framed.len(), message_len, "gRPC frame length should match");
        T::decode(framed).expect("protobuf payload should decode")
    }

    fn test_log_sender() -> mpsc::Sender<AsyncLogEntry> {
        disconnected_log_sender()
    }

    fn build_test_engine(config: &IntegrityConfig) -> Engine {
        build_engine(config, false).expect("engine should be created")
    }

    fn build_test_metered_engine(config: &IntegrityConfig) -> Engine {
        build_engine(config, true).expect("metered engine should be created")
    }

    fn build_test_runtime(config: IntegrityConfig) -> RuntimeState {
        build_runtime_state(config).expect("runtime state should build")
    }

    #[cfg(feature = "ai-inference")]
    fn test_ai_runtime(config: &IntegrityConfig) -> Arc<ai_inference::AiInferenceRuntime> {
        Arc::new(
            ai_inference::AiInferenceRuntime::from_config(config)
                .expect("AI inference runtime should build"),
        )
    }

    #[derive(Debug)]
    struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> std::result::Result<ServerCertVerified, RustlsError> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &DigitallySignedStruct,
        ) -> std::result::Result<HandshakeSignatureValid, RustlsError> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::RSA_PSS_SHA256,
            ]
        }
    }

    fn insecure_tls_connector() -> TlsConnector {
        TlsConnector::from(Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
                .with_no_client_auth(),
        ))
    }

    fn signed_manifest(config: &IntegrityConfig, seed: u8) -> IntegrityManifest {
        let config_payload = canonical_config_payload(config).expect("payload should serialize");
        let signing_key = SigningKey::from_bytes(&[seed; 32]);
        let signature = signing_key.sign(&Sha256::digest(config_payload.as_bytes()));

        IntegrityManifest {
            config_payload,
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
        }
    }

    fn test_host_identity(seed: u8) -> Arc<HostIdentity> {
        Arc::new(HostIdentity::from_signing_key(SigningKey::from_bytes(
            &[seed; 32],
        )))
    }

    fn write_test_manifest(path: &Path, config: &IntegrityConfig, seed: u8) {
        let manifest = signed_manifest(config, seed);
        fs::write(
            path,
            serde_json::to_string_pretty(&manifest).expect("manifest should serialize"),
        )
        .expect("manifest should be written");
    }

    fn autoscaling_test_config(include_background_route: bool) -> IntegrityConfig {
        let mut routes = vec![
            IntegrityRoute::user("/api/guest-call-legacy"),
            IntegrityRoute::system("/metrics/scaling"),
        ];
        if include_background_route {
            routes.push(IntegrityRoute::system("/system/k8s-scaler"));
        }

        IntegrityConfig {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            routes,
        }
    }

    fn sqs_connector_test_config(
        host_address: SocketAddr,
        queue_url: String,
        target_route_path: &str,
        target_module: &str,
    ) -> IntegrityConfig {
        let mut target_route = IntegrityRoute::user(target_route_path);
        target_route.targets = vec![RouteTarget {
            module: target_module.to_owned(),
            weight: 100,
            websocket: false,
            match_header: None,
        }];

        let mut connector_route = IntegrityRoute::system("/system/sqs-connector");
        connector_route.name = "sqs-connector".to_owned();
        connector_route.targets = vec![RouteTarget {
            module: "system-faas-sqs".to_owned(),
            weight: 100,
            websocket: false,
            match_header: None,
        }];
        connector_route.dependencies = BTreeMap::from([(
            default_route_name(target_route_path),
            default_route_version(),
        )]);
        connector_route.env = BTreeMap::from([
            ("QUEUE_URL".to_owned(), queue_url),
            ("TARGET_ROUTE".to_owned(), target_route_path.to_owned()),
        ]);

        IntegrityConfig {
            host_address: host_address.to_string(),
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            routes: vec![target_route, connector_route],
        }
    }

    fn gc_batch_target(cache_dir: &Path, ttl_seconds: u64) -> IntegrityBatchTarget {
        IntegrityBatchTarget {
            name: "gc-job".to_owned(),
            module: "system-faas-gc".to_owned(),
            env: BTreeMap::from([
                ("TARGET_DIR".to_owned(), "/cache".to_owned()),
                ("TTL_SECONDS".to_owned(), ttl_seconds.to_string()),
            ]),
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: cache_dir.display().to_string(),
                guest_path: "/cache".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn build_test_state(config: IntegrityConfig, telemetry: TelemetryHandle) -> AppState {
        build_test_state_with_manifest(
            config,
            telemetry,
            unique_test_dir("integrity-manifest").join("integrity.lock"),
        )
    }

    fn build_test_state_with_manifest(
        config: IntegrityConfig,
        telemetry: TelemetryHandle,
        manifest_path: PathBuf,
    ) -> AppState {
        let (async_log_sender, async_log_receiver) = mpsc::channel(LOG_QUEUE_CAPACITY);
        let buffered_requests = Arc::new(BufferedRequestManager::new(buffered_request_spool_dir(
            &manifest_path,
        )));
        let core_store = Arc::new(
            store::CoreStore::open(&core_store_path(&manifest_path))
                .expect("test core store should open"),
        );
        let state = AppState {
            runtime: Arc::new(ArcSwap::from_pointee(build_test_runtime(config))),
            draining_runtimes: Arc::new(Mutex::new(Vec::new())),
            http_client: Client::new(),
            async_log_sender,
            secrets_vault: SecretsVault::load(),
            host_identity: test_host_identity(21),
            uds_fast_path: Arc::new(new_uds_fast_path_registry()),
            storage_broker: Arc::new(StorageBrokerManager::new(Arc::clone(&core_store))),
            core_store,
            buffered_requests,
            volume_manager: Arc::new(VolumeManager::default()),
            telemetry,
            tls_manager: Arc::new(tls_runtime::TlsManager::default()),
            manifest_path,
            background_workers: Arc::new(BackgroundWorkerManager::default()),
        };
        let runtime = state.runtime.load_full();
        prewarm_runtime_routes(
            &runtime,
            state.telemetry.clone(),
            Arc::clone(&state.host_identity),
            Arc::clone(&state.storage_broker),
        )
        .expect("test runtime should prewarm successfully");
        drop(runtime);
        spawn_async_log_exporter(state.clone(), async_log_receiver);
        if tokio::runtime::Handle::try_current().is_ok() {
            spawn_buffered_request_replayer(state.clone());
            spawn_pressure_monitor(state.clone());
        }
        state
    }

    fn volume_test_route(host_path: &std::path::Path, readonly: bool) -> IntegrityRoute {
        IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn logger_test_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: SYSTEM_LOGGER_ROUTE.to_owned(),
            role: RouteRole::System,
            name: "system-faas-logger".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn log_storm_test_route() -> IntegrityRoute {
        let mut route = IntegrityRoute::user("/api/guest-log-storm");
        route.name = "guest-log-storm".to_owned();
        route
    }

    fn execute_legacy_guest_with_sync_file_capture(
        engine: &Engine,
        function_name: &str,
        body: Bytes,
        route: &IntegrityRoute,
        execution: &GuestExecutionContext,
    ) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
        let (module_path, module) = resolve_legacy_guest_module(
            engine,
            function_name,
            &execution.storage_broker.core_store,
            "default",
        )?;
        let linker = build_linker(engine)?;
        let stdin_file = create_guest_stdin_file(&body)?;
        let stdout_file = create_guest_stdout_file()?;
        let stdout_path = stdout_file.path.clone();
        let mut wasi = WasiCtxBuilder::new();
        wasi.arg(legacy_guest_program_name(&module_path))
            .stdin(InputFile::new(stdin_file.file.try_clone().map_err(
                |error| {
                    guest_execution_error(error.into(), "failed to clone guest stdin file handle")
                },
            )?))
            .stderr(AsyncGuestOutputCapture::new(
                format!("{function_name}-sync-benchmark"),
                GuestLogStreamType::Stderr,
                disconnected_log_sender(),
                false,
                0,
            ));

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

        preopen_route_volumes(&mut wasi, route)?;

        let stdout_clone = stdout_file.file.try_clone().map_err(|error| {
            guest_execution_error(
                error.into(),
                "failed to clone sync benchmark stdout file handle",
            )
        })?;
        wasi.stdout(OutputFile::new(stdout_clone));
        let wasi = wasi.build_p1();
        let mut store = Store::new(
            engine,
            LegacyHostState::new(
                wasi,
                execution.config.guest_memory_limit_bytes,
                #[cfg(feature = "ai-inference")]
                Arc::clone(&execution.ai_runtime),
            ),
        );
        store.limiter(|state| &mut state.limits);
        maybe_set_guest_fuel_budget(&mut store, execution)?;
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|error| guest_execution_error(error, "failed to instantiate guest module"))?;
        let (entrypoint_name, entrypoint) = resolve_guest_entrypoint(&mut store, &instance)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    "failed to resolve exported function `faas_entry` or `_start`",
                )
            })?;

        let call_result = entrypoint.call(&mut store, ());
        let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
        handle_guest_entrypoint_result(entrypoint_name, call_result)?;
        stdout_file.file.sync_all().map_err(|error| {
            guest_execution_error(
                error.into(),
                "failed to flush guest stdout temp file to disk",
            )
        })?;
        let stdout_bytes = read_guest_stdout_file(&stdout_path, execution.config.max_stdout_bytes)?;

        Ok(GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(split_guest_stdout(
                function_name,
                stdout_bytes,
            )),
            fuel_consumed,
        })
    }

    fn scoped_volume_test_route(
        path: &str,
        host_path: &std::path::Path,
        guest_path: &str,
        readonly: bool,
    ) -> IntegrityRoute {
        IntegrityRoute {
            path: path.to_owned(),
            role: RouteRole::User,
            name: default_route_name(path),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: guest_path.to_owned(),
                readonly,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn storage_broker_test_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: "/system/storage-broker".to_owned(),
            role: RouteRole::System,
            name: "storage-broker".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: vec![RouteTarget {
                module: "system-faas-storage-broker".to_owned(),
                weight: 100,
                websocket: false,
                match_header: None,
            }],
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn metering_test_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: SYSTEM_METERING_ROUTE.to_owned(),
            role: RouteRole::System,
            name: "metering".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: vec![RouteTarget {
                module: "system-faas-metering".to_owned(),
                weight: 100,
                websocket: false,
                match_header: None,
            }],
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn cert_manager_test_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: SYSTEM_CERT_MANAGER_ROUTE.to_owned(),
            role: RouteRole::System,
            name: "cert-manager".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: vec![RouteTarget {
                module: "system-faas-cert-manager".to_owned(),
                weight: 100,
                websocket: false,
                match_header: None,
            }],
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: CERT_MANAGER_GUEST_CERT_DIR.to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn tcp_echo_test_route(max_concurrency: u32) -> IntegrityRoute {
        IntegrityRoute {
            path: "/tcp/echo".to_owned(),
            role: RouteRole::User,
            name: "guest-tcp-echo".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency,
            volumes: Vec::new(),
        }
    }

    fn udp_echo_test_route(max_concurrency: u32) -> IntegrityRoute {
        IntegrityRoute {
            path: "/udp/echo".to_owned(),
            role: RouteRole::User,
            name: "guest-udp-echo".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency,
            volumes: Vec::new(),
        }
    }

    fn free_tcp_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("temporary TCP listener should bind")
            .local_addr()
            .expect("temporary TCP listener should expose an address")
            .port()
    }

    fn free_udp_port() -> u16 {
        std::net::UdpSocket::bind("127.0.0.1:0")
            .expect("temporary UDP socket should bind")
            .local_addr()
            .expect("temporary UDP socket should expose an address")
            .port()
    }

    fn hibernating_ram_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Ram,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: Some("50ms".to_owned()),
                eviction_policy: Some(VolumeEvictionPolicy::Hibernate),
            }],
        }
    }

    fn ttl_managed_volume_route(host_path: &std::path::Path, ttl_seconds: u64) -> IntegrityRoute {
        IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: host_path.display().to_string(),
                guest_path: "/app/data".to_owned(),
                readonly: true,
                ttl_seconds: Some(ttl_seconds),
                idle_timeout: None,
                eviction_policy: None,
            }],
        }
    }

    fn resiliency_test_route(resiliency: Option<ResiliencyConfig>) -> IntegrityRoute {
        let mut route = IntegrityRoute::user("/api/guest-flaky");
        route.name = "guest-flaky".to_owned();
        route.resiliency = resiliency;
        route
    }

    fn draining_test_route(module: &str, version: &str) -> IntegrityRoute {
        let mut route = targeted_route("/api/drain", vec![weighted_target(module, 100)]);
        route.name = "guest-drain".to_owned();
        route.version = version.to_owned();
        route
    }

    fn targeted_route(path: &str, targets: Vec<RouteTarget>) -> IntegrityRoute {
        let mut route = IntegrityRoute::user(path);
        route.targets = targets;
        route
    }

    fn versioned_route(path: &str, name: &str, version: &str) -> IntegrityRoute {
        let mut route = IntegrityRoute::user(path);
        route.name = name.to_owned();
        route.version = version.to_owned();
        route
    }

    fn dependency_route(
        path: &str,
        name: &str,
        version: &str,
        dependencies: &[(&str, &str)],
    ) -> IntegrityRoute {
        let mut route = versioned_route(path, name, version);
        route.dependencies = dependencies
            .iter()
            .map(|(dependency, requirement)| ((*dependency).to_owned(), (*requirement).to_owned()))
            .collect();
        route
    }

    fn weighted_target(module: &str, weight: u32) -> RouteTarget {
        RouteTarget {
            module: module.to_owned(),
            weight,
            websocket: false,
            match_header: None,
        }
    }

    fn header_target(module: &str, header_name: &str, header_value: &str) -> RouteTarget {
        RouteTarget {
            module: module.to_owned(),
            weight: 0,
            websocket: false,
            match_header: Some(HeaderMatch {
                name: header_name.to_owned(),
                value: header_value.to_owned(),
            }),
        }
    }

    fn websocket_target(module: &str) -> RouteTarget {
        RouteTarget {
            module: module.to_owned(),
            weight: 100,
            websocket: true,
            match_header: None,
        }
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let short_prefix: String = prefix
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .take(8)
            .collect();
        let short_prefix = if short_prefix.is_empty() {
            "tmp".to_owned()
        } else {
            short_prefix.to_ascii_lowercase()
        };
        let unique_id = Uuid::new_v4().simple().to_string();
        let path = std::env::temp_dir().join(format!("{short_prefix}-{}", &unique_id[..8]));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        path
    }

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
    fn system_runtime_environment_only_exposes_host_public_key_to_system_routes() {
        let host_identity = test_host_identity(12);
        let mut system_route = IntegrityRoute::system("/system/storage-broker");
        system_route
            .env
            .insert("QUEUE_URL".to_owned(), "http://queue.local/mock".to_owned());
        let system_env = system_runtime_environment(&system_route, &host_identity);
        let user_env =
            system_runtime_environment(&IntegrityRoute::user("/api/guest"), &host_identity);

        assert_eq!(
            system_env,
            vec![
                ("QUEUE_URL".to_owned(), "http://queue.local/mock".to_owned()),
                (
                    TACHYON_SYSTEM_PUBLIC_KEY_ENV.to_owned(),
                    host_identity.public_key_hex.clone()
                )
            ]
        );
        assert!(user_env.is_empty());
    }

    #[test]
    fn host_identity_round_trips_signed_route_claims() {
        let host_identity = test_host_identity(13);
        let route = IntegrityRoute::user("/api/tenant-a");
        let token = host_identity
            .sign_route(&route)
            .expect("identity token should sign");

        let claims = host_identity
            .verify_token(&token)
            .expect("identity token should verify");

        assert_eq!(claims.route_path, "/api/tenant-a");
        assert_eq!(claims.role, RouteRole::User);
        assert!(claims.expires_at >= claims.issued_at);
    }

    #[test]
    fn host_identity_rejects_expired_tokens() {
        let host_identity = test_host_identity(14);
        let now = unix_timestamp_seconds().expect("system clock should be available");
        let token = host_identity
            .sign_claims(&CallerIdentityClaims {
                route_path: "/api/tenant-a".to_owned(),
                role: RouteRole::User,
                issued_at: now.saturating_sub(10),
                expires_at: now.saturating_sub(1),
            })
            .expect("expired identity token should still sign");

        let error = host_identity
            .verify_token(&token)
            .expect_err("expired identity token should be rejected");

        assert!(error.contains("expired"), "unexpected error: {error}");
    }

    #[tokio::test]
    async fn reload_runtime_from_disk_swaps_in_new_routes() {
        let temp_dir = unique_test_dir("graceful-reload-state");
        let manifest_path = temp_dir.join("integrity.lock");
        let initial = IntegrityConfig {
            routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
            ..IntegrityConfig::default_sealed()
        };
        write_test_manifest(&manifest_path, &initial, 11);
        let state = build_test_state_with_manifest(
            initial,
            telemetry::init_test_telemetry(),
            manifest_path.clone(),
        );

        let mut reloaded = IntegrityConfig {
            routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
            ..IntegrityConfig::default_sealed()
        };
        reloaded
            .routes
            .push(IntegrityRoute::user("/api/guest-loop"));
        write_test_manifest(&manifest_path, &reloaded, 12);

        reload_runtime_from_disk(&state)
            .await
            .expect("runtime should reload from manifest");

        let runtime = state.runtime.load_full();
        assert!(runtime.config.sealed_route("/api/guest-loop").is_some());
        assert!(runtime.concurrency_limits.contains_key("/api/guest-loop"));
    }

    #[tokio::test]
    async fn reload_runtime_from_disk_keeps_previous_state_on_invalid_manifest() {
        let temp_dir = unique_test_dir("graceful-reload-invalid");
        let manifest_path = temp_dir.join("integrity.lock");
        let initial = IntegrityConfig {
            routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
            ..IntegrityConfig::default_sealed()
        };
        write_test_manifest(&manifest_path, &initial, 13);
        let state = build_test_state_with_manifest(
            initial,
            telemetry::init_test_telemetry(),
            manifest_path.clone(),
        );

        fs::write(&manifest_path, "{ invalid json").expect("invalid manifest should be written");

        let error = reload_runtime_from_disk(&state)
            .await
            .expect_err("invalid manifest should not replace the runtime");

        assert!(error
            .to_string()
            .contains("failed to parse integrity manifest"));
        let runtime = state.runtime.load_full();
        assert!(runtime.config.sealed_route(DEFAULT_ROUTE).is_some());
        assert!(runtime.config.sealed_route("/api/guest-loop").is_none());
    }

    #[tokio::test]
    async fn reload_runtime_from_disk_drains_previous_generation_until_response_flush() {
        let temp_dir = unique_test_dir("graceful-drain");
        let manifest_path = temp_dir.join("integrity.lock");
        let initial = IntegrityConfig {
            routes: vec![draining_test_route("guest-flaky", "1.0.0")],
            ..IntegrityConfig::default_sealed()
        };
        write_test_manifest(&manifest_path, &initial, 31);
        let state = build_test_state_with_manifest(
            initial,
            telemetry::init_test_telemetry(),
            manifest_path.clone(),
        );
        let app = build_app(state.clone());

        let slow_request = {
            let app = app.clone();
            tokio::spawn(async move {
                app.oneshot(
                    Request::post("/api/drain")
                        .body(Body::from("sleep:250"))
                        .expect("slow request should build"),
                )
                .await
                .expect("slow request should complete")
            })
        };

        tokio::time::sleep(Duration::from_millis(50)).await;
        let initial_runtime = state.runtime.load_full();
        let initial_control = initial_runtime
            .concurrency_limits
            .get("/api/drain")
            .cloned()
            .expect("initial route control should exist");
        assert_eq!(initial_control.active_request_count(), 1);
        drop(initial_runtime);

        let reloaded = IntegrityConfig {
            routes: vec![draining_test_route("guest-example", "2.0.0")],
            ..IntegrityConfig::default_sealed()
        };
        write_test_manifest(&manifest_path, &reloaded, 32);
        reload_runtime_from_disk(&state)
            .await
            .expect("runtime should reload from manifest");

        let fresh_response = app
            .clone()
            .oneshot(
                Request::post("/api/drain")
                    .body(Body::from("hello-v2"))
                    .expect("fresh request should build"),
            )
            .await
            .expect("fresh request should complete");
        let fresh_body = fresh_response
            .into_body()
            .collect()
            .await
            .expect("fresh response body should collect")
            .to_bytes();
        assert!(
            String::from_utf8_lossy(&fresh_body).contains("FaaS received: hello-v2"),
            "unexpected fresh body: {:?}",
            fresh_body
        );

        let slow_response = slow_request
            .await
            .expect("slow request task should join cleanly");
        assert_eq!(
            state
                .draining_runtimes
                .lock()
                .expect("draining runtime list should not be poisoned")
                .len(),
            1
        );
        run_draining_runtime_reaper_tick(&state);
        assert_eq!(
            state
                .draining_runtimes
                .lock()
                .expect("draining runtime list should not be poisoned")
                .len(),
            1,
            "the old generation should remain while its response body is still owned"
        );
        assert_eq!(initial_control.active_request_count(), 1);

        let slow_body = slow_response
            .into_body()
            .collect()
            .await
            .expect("slow response body should collect")
            .to_bytes();
        assert_eq!(slow_body, Bytes::from_static(b"slept:250"));
        run_draining_runtime_reaper_tick(&state);
        assert_eq!(initial_control.active_request_count(), 0);
        assert_eq!(
            state
                .draining_runtimes
                .lock()
                .expect("draining runtime list should not be poisoned")
                .len(),
            0,
            "the old generation should be reaped once the response finishes flushing"
        );
    }

    #[test]
    fn draining_runtime_reaper_forces_timeout_after_deadline() {
        let state = build_test_state(
            IntegrityConfig {
                routes: vec![draining_test_route("guest-flaky", "1.0.0")],
                ..IntegrityConfig::default_sealed()
            },
            telemetry::init_test_telemetry(),
        );
        let runtime = state.runtime.load_full();
        let control = runtime
            .concurrency_limits
            .get("/api/drain")
            .cloned()
            .expect("route control should exist");
        let _guard = control.begin_request();
        let draining_since = Instant::now()
            .checked_sub(DRAINING_ROUTE_TIMEOUT + Duration::from_secs(1))
            .expect("deadline subtraction should remain valid");
        runtime.mark_draining(draining_since);
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .push(DrainingRuntime {
                runtime,
                draining_since,
            });

        run_draining_runtime_reaper_tick(&state);

        assert_eq!(
            state
                .draining_runtimes
                .lock()
                .expect("draining runtime list should not be poisoned")
                .len(),
            0
        );
        assert!(control.semaphore.is_closed());
    }

    #[tokio::test]
    async fn run_mode_executes_gc_batch_target_and_deletes_stale_files() {
        let temp_dir = unique_test_dir("batch-gc");
        let cache_dir = temp_dir.join("cache");
        fs::create_dir_all(cache_dir.join("nested")).expect("cache directory should exist");
        let stale_file = cache_dir.join("nested").join("stale.txt");
        fs::write(&stale_file, "stale").expect("stale file should be written");

        let manifest_path = temp_dir.join("integrity.lock");
        let mut config = IntegrityConfig::default_sealed();
        config.routes.clear();
        config.batch_targets = vec![gc_batch_target(&cache_dir, 0)];
        write_test_manifest(&manifest_path, &config, 14);

        let success = execute_batch_target_from_manifest(manifest_path, "gc-job")
            .await
            .expect("batch target should execute successfully");

        assert!(success, "batch target should exit successfully");
        assert!(
            !stale_file.exists(),
            "batch GC target should delete stale files"
        );
    }

    #[test]
    fn execute_guest_returns_component_response_payload() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = config
            .sealed_route("/api/guest-example")
            .expect("sealed route should exist")
            .clone();
        #[cfg(feature = "ai-inference")]
        let ai_runtime = test_ai_runtime(&config);
        let response = execute_guest(
            &engine,
            "guest-example",
            GuestRequest::new("POST", "/api/guest-example", "Hello Lean FaaS!"),
            &route,
            GuestExecutionContext {
                secret_access: SecretAccess::from_route(&route, &SecretsVault::load()),
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(30),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            },
        )
        .expect("guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                    StatusCode::OK,
                    Bytes::from(expected_guest_example_body(
                        "FaaS received: Hello Lean FaaS!"
                    )),
                )),
                fuel_consumed: None,
            }
        );
    }

    #[test]
    fn execute_guest_falls_back_to_legacy_stdout_for_non_component_module() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = IntegrityRoute::user("/api/guest-call-legacy");
        #[cfg(feature = "ai-inference")]
        let ai_runtime = test_ai_runtime(&config);
        let response = execute_guest(
            &engine,
            "guest-call-legacy",
            GuestRequest::new("GET", "/api/guest-call-legacy", Bytes::new()),
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(31),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            },
        )
        .expect("legacy guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::LegacyStdout(Bytes::from(
                    "MESH_FETCH:http://legacy-service:8081/ping\n"
                )),
                fuel_consumed: None,
            }
        );
    }

    #[test]
    fn execute_legacy_guest_reads_stdin_for_tcp_echo_module() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = tcp_echo_test_route(1);
        #[cfg(feature = "ai-inference")]
        let ai_runtime = test_ai_runtime(&config);
        let response = execute_guest(
            &engine,
            "guest-tcp-echo",
            GuestRequest::new(
                "TCP",
                "tcp://guest-tcp-echo",
                Bytes::from_static(b"ping over tcp"),
            ),
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(32),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            },
        )
        .expect("legacy guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::LegacyStdout(Bytes::from_static(b"ping over tcp")),
                fuel_consumed: None,
            }
        );
    }

    #[cfg(feature = "ai-inference")]
    #[test]
    fn execute_guest_ai_uses_preloaded_model_alias_and_returns_mock_text() {
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: "llama3".to_owned(),
            path: "/models/llama3.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::Standard,
        }];
        let config = IntegrityConfig {
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        };
        let engine = build_test_engine(&config);
        let ai_runtime = test_ai_runtime(&config);

        let response = execute_guest(
            &engine,
            "guest-ai",
            GuestRequest::new(
                "POST",
                "/api/guest-ai",
                Bytes::from_static(
                    br#"{"model":"llama3","shape":[1,4],"values":[1.0,2.0,3.0,4.0],"output_len":17,"response_kind":"text"}"#,
                ),
            ),
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(35),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                ai_runtime,
            },
        )
        .expect("AI guest execution should succeed");

        let GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(stdout),
            ..
        } = response
        else {
            panic!("AI guest should return legacy stdout");
        };

        let payload: Value =
            serde_json::from_slice(&stdout).expect("guest response should be JSON");
        assert_eq!(payload["model"], Value::String("llama3".to_owned()));
        assert_eq!(
            payload["text"],
            Value::String("MOCK_LLM_RESPONSE".to_owned())
        );
        assert_eq!(payload["output_bytes"], Value::from(17));
    }

    #[test]
    fn execute_guest_persists_volume_data_for_component_guest() {
        let volume_dir = unique_test_dir("tachyon-volume-test");
        let route = volume_test_route(&volume_dir, false);
        let config = IntegrityConfig {
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        };
        let engine = build_test_engine(&config);
        #[cfg(feature = "ai-inference")]
        let ai_runtime = test_ai_runtime(&config);

        let save_response = execute_guest(
            &engine,
            "guest-volume",
            GuestRequest::new("POST", "/api/guest-volume", "Hello Stateful World"),
            &route,
            GuestExecutionContext {
                config: config.clone(),
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(32),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime: Arc::clone(&ai_runtime),
            },
        )
        .expect("volume guest should write successfully");

        assert_eq!(
            save_response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::Http(
                    GuestHttpResponse::new(StatusCode::OK, "Saved",)
                ),
                fuel_consumed: None,
            }
        );

        let read_response = execute_guest(
            &engine,
            "guest-volume",
            GuestRequest::new("GET", "/api/guest-volume", Bytes::new()),
            &route,
            GuestExecutionContext {
                config: config.clone(),
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(33),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            },
        )
        .expect("volume guest should read successfully");

        assert_eq!(
            read_response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                    StatusCode::OK,
                    "Hello Stateful World",
                )),
                fuel_consumed: None,
            }
        );
        assert_eq!(
            fs::read_to_string(volume_dir.join("state.txt"))
                .expect("host volume file should exist"),
            "Hello Stateful World"
        );

        let _ = fs::remove_dir_all(volume_dir);
    }

    #[tokio::test]
    async fn router_returns_guest_stdout_for_post_request() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));
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
            expected_guest_example_body("FaaS received: Hello Lean FaaS!")
        );
    }

    #[tokio::test]
    async fn router_accepts_get_requests() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));
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
            expected_guest_example_body("FaaS received an empty payload")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn grpc_http2_route_returns_protobuf_body_and_grpc_status_trailer() {
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![targeted_route(
                "/grpc/hello",
                vec![weighted_target("guest-grpc", 100)],
            )],
            ..IntegrityConfig::default_sealed()
        })
        .expect("gRPC route config should validate");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("gRPC test listener should bind");
        let address = listener
            .local_addr()
            .expect("gRPC test listener should expose an address");
        let server = tokio::spawn(async move {
            serve_http_listener(listener, app)
                .await
                .expect("gRPC test server should stay healthy");
        });

        let stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("gRPC client should connect");
        let (mut sender, connection) =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
                .await
                .expect("HTTP/2 handshake should succeed");
        let connection_task = tokio::spawn(async move {
            connection
                .await
                .expect("HTTP/2 connection should stay healthy");
        });

        let request_body = encode_test_grpc_message(&TestGrpcHelloRequest {
            name: "Tachyon".to_owned(),
        });
        let response = sender
            .send_request(
                Request::builder()
                    .method("POST")
                    .uri(format!("http://{address}/grpc/hello"))
                    .version(hyper::Version::HTTP_2)
                    .header("content-type", "application/grpc")
                    .header("te", "trailers")
                    .body(Full::new(Bytes::from(request_body)))
                    .expect("gRPC request should build"),
            )
            .await
            .expect("gRPC request should complete");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.version(), hyper::Version::HTTP_2);
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("application/grpc")
        );

        let mut body = response.into_body();
        let mut framed_payload = Vec::new();
        let mut trailers = None;
        while let Some(frame) = body.frame().await {
            let frame = frame.expect("HTTP/2 response frame should be readable");
            if let Some(data) = frame.data_ref() {
                framed_payload.extend_from_slice(data);
            }
            if let Some(frame_trailers) = frame.trailers_ref() {
                trailers = Some(frame_trailers.clone());
            }
        }

        let decoded = decode_test_grpc_message::<TestGrpcHelloResponse>(&framed_payload);
        assert_eq!(decoded.message, "Hello, Tachyon!");
        assert_eq!(
            trailers
                .as_ref()
                .and_then(|trailers| trailers.get("grpc-status"))
                .and_then(|value| value.to_str().ok()),
            Some("0")
        );

        server.abort();
        connection_task.abort();
        let _ = server.await;
        let _ = connection_task.await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn async_logger_exports_log_storm_without_leaking_logs_into_response() {
        let log_dir = unique_test_dir("tachyon-async-logger");
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![log_storm_test_route(), logger_test_route(&log_dir)],
            ..IntegrityConfig::default_sealed()
        })
        .expect("async logger config should validate");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

        let response = app
            .oneshot(
                Request::post("/api/guest-log-storm")
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
        assert_eq!(String::from_utf8_lossy(&body).trim(), "storm-complete");

        let log_file = log_dir.join("guest-logs.ndjson");
        for _ in 0..30 {
            if log_file.exists()
                && fs::metadata(&log_file)
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        let contents = fs::read_to_string(&log_file).expect("logger output should exist");
        assert!(contents.contains("\"target_name\":\"guest-log-storm\""));
        assert!(contents.contains("\"message\":\"storm-"));

        let _ = fs::remove_dir_all(log_dir);
    }

    #[test]
    #[ignore = "manual performance comparison for async logging validation"]
    fn async_log_capture_is_faster_than_sync_file_capture() {
        let route = log_storm_test_route();
        let config = validate_integrity_config(IntegrityConfig {
            max_stdout_bytes: 16 * 1024 * 1024,
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        })
        .expect("benchmark config should validate");
        let engine = build_test_engine(&config);
        let async_execution = GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(44),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: test_ai_runtime(&config),
        };

        let request = GuestRequest::new("POST", "/api/guest-log-storm", Bytes::new());

        let async_start = Instant::now();
        let async_result = execute_guest(
            &engine,
            "guest-log-storm",
            request.clone(),
            &route,
            async_execution,
        )
        .expect("async log capture should succeed");
        let async_elapsed = async_start.elapsed();

        let sync_execution = GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(45),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: test_ai_runtime(&config),
        };
        let sync_start = Instant::now();
        let sync_result = execute_legacy_guest_with_sync_file_capture(
            &engine,
            "guest-log-storm",
            request.body,
            &route,
            &sync_execution,
        )
        .expect("sync log capture should succeed");
        let sync_elapsed = sync_start.elapsed();

        let GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(async_stdout),
            ..
        } = async_result
        else {
            panic!("async benchmark should return legacy stdout");
        };
        let GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(sync_stdout),
            ..
        } = sync_result
        else {
            panic!("sync benchmark should return legacy stdout");
        };

        assert_eq!(
            String::from_utf8_lossy(&async_stdout).trim(),
            "storm-complete"
        );
        assert_eq!(
            String::from_utf8_lossy(&sync_stdout).trim(),
            "storm-complete"
        );
        eprintln!(
            "guest-log-storm benchmark: async_capture={async_elapsed:?}, sync_file_capture={sync_elapsed:?}"
        );
        assert!(
            async_elapsed < sync_elapsed,
            "expected async capture to beat sync file capture (async={async_elapsed:?}, sync={sync_elapsed:?})"
        );
    }

    #[tokio::test]
    async fn router_rejects_exhausted_hop_limit_header() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));
        let response = app
            .oneshot(
                Request::get("/api/guest-example")
                    .header(HOP_LIMIT_HEADER, "0")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::LOOP_DETECTED);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();

        assert!(
            String::from_utf8_lossy(&body).contains("Routing loop detected"),
            "unexpected loop-detected response body: {:?}",
            body
        );
    }

    #[tokio::test]
    async fn router_rejects_unsealed_routes() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));
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
    async fn router_returns_service_unavailable_when_route_concurrency_is_exhausted() {
        let config = IntegrityConfig::default_sealed();
        let runtime = RuntimeState {
            engine: build_test_engine(&config),
            metered_engine: build_test_metered_engine(&config),
            route_registry: Arc::new(
                RouteRegistry::build(&config).expect("route registry should build"),
            ),
            batch_target_registry: Arc::new(
                BatchTargetRegistry::build(&config).expect("batch target registry should build"),
            ),
            concurrency_limits: Arc::new(HashMap::from([(
                DEFAULT_ROUTE.to_owned(),
                Arc::new(RouteExecutionControl::from_limits(0, 0)),
            )])),
            #[cfg(feature = "ai-inference")]
            ai_runtime: test_ai_runtime(&config),
            config,
        };
        let core_store_manifest = unique_test_dir("app-state-manifest").join("integrity.lock");
        let buffered_requests = Arc::new(BufferedRequestManager::new(buffered_request_spool_dir(
            &core_store_manifest,
        )));
        let core_store = Arc::new(
            store::CoreStore::open(&core_store_path(&core_store_manifest))
                .expect("test core store should open"),
        );
        let state = AppState {
            runtime: Arc::new(ArcSwap::from_pointee(runtime)),
            draining_runtimes: Arc::new(Mutex::new(Vec::new())),
            http_client: Client::new(),
            async_log_sender: test_log_sender(),
            secrets_vault: SecretsVault::load(),
            host_identity: test_host_identity(22),
            uds_fast_path: Arc::new(new_uds_fast_path_registry()),
            storage_broker: Arc::new(StorageBrokerManager::new(Arc::clone(&core_store))),
            core_store,
            buffered_requests,
            volume_manager: Arc::new(VolumeManager::default()),
            telemetry: telemetry::init_test_telemetry(),
            tls_manager: Arc::new(tls_runtime::TlsManager::default()),
            manifest_path: core_store_manifest,
            background_workers: Arc::new(BackgroundWorkerManager::default()),
        };
        spawn_buffered_request_replayer(state.clone());
        spawn_pressure_monitor(state.clone());
        let app = build_app(state);

        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("blocked"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = response
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();

        assert!(String::from_utf8_lossy(&body).contains("buffered request timed out"));
    }

    #[test]
    fn buffered_request_manager_spills_to_disk_after_ram_capacity() {
        let spool_dir = unique_test_dir("buffered-request-spool");
        let manager = BufferedRequestManager::new(spool_dir);
        let request = BufferedRouteRequest {
            route_path: DEFAULT_ROUTE.to_owned(),
            selected_module: "guest-example".to_owned(),
            method: "POST".to_owned(),
            uri: "http://localhost/api/guest-example".to_owned(),
            headers: Vec::new(),
            body: b"payload".to_vec(),
            trailers: Vec::new(),
            hop_limit: DEFAULT_HOP_LIMIT,
            trace_id: None,
            sampled_execution: false,
        };

        for _ in 0..=BUFFER_RAM_REQUEST_CAPACITY {
            let _ = manager
                .enqueue(request.clone())
                .expect("buffered request should enqueue");
        }

        assert_eq!(manager.pending_count(), BUFFER_RAM_REQUEST_CAPACITY + 1);
        assert_eq!(manager.disk_spill_count(), 1);
    }

    #[test]
    fn system_guest_requires_system_route_role() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = IntegrityRoute::user("/metrics");
        #[cfg(feature = "ai-inference")]
        let ai_runtime = test_ai_runtime(&config);
        let error = execute_guest(
            &engine,
            "metrics",
            GuestRequest::new("GET", "/metrics", Bytes::new()),
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(34),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            },
        )
        .expect_err("privileged metrics guest should fail as a user route");

        match error {
            ExecutionError::Internal(message) => {
                assert!(
                    message.contains("telemetry-reader") || message.contains("telemetry_reader")
                );
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn router_returns_system_metrics_for_privileged_route() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));

        let response = app
            .oneshot(
                Request::get("/metrics")
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
        let text = String::from_utf8_lossy(&body);

        assert!(text.contains("tachyon_requests_total"));
        assert!(text.contains("tachyon_active_requests"));
    }

    #[tokio::test]
    async fn router_returns_scaling_metrics_for_privileged_route() {
        let config = autoscaling_test_config(false);
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let runtime = state.runtime.load_full();
        runtime
            .concurrency_limits
            .get("/api/guest-call-legacy")
            .expect("legacy route should have a limiter")
            .pending_waiters
            .store(7, Ordering::SeqCst);
        let app = build_app(state);

        let response = app
            .oneshot(
                Request::get("/metrics/scaling")
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
        let text = String::from_utf8_lossy(&body);

        assert!(text.contains("tachyon_pending_requests"));
        assert!(text.contains("route=\"/api/guest-call-legacy\""));
        assert!(text.contains(" 7"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_scaler_tick_respects_cooldown() {
        use axum::{extract::State, routing::patch, Router};
        use std::sync::Mutex;

        async fn capture_patch(
            State(captured): State<Arc<Mutex<Vec<String>>>>,
            body: Bytes,
        ) -> StatusCode {
            captured
                .lock()
                .expect("captured requests should not be poisoned")
                .push(String::from_utf8_lossy(&body).into_owned());
            StatusCode::OK
        }

        let captured = Arc::new(Mutex::new(Vec::new()));
        let mock_app = Router::new()
            .route(
                "/apis/apps/v1/namespaces/default/deployments/legacy-app",
                patch(capture_patch),
            )
            .with_state(Arc::clone(&captured));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener
            .local_addr()
            .expect("mock server should expose a local address");
        let server = tokio::spawn(async move {
            axum::serve(listener, mock_app)
                .await
                .expect("mock server should stay up");
        });

        std::env::set_var(MOCK_K8S_URL_ENV, format!("http://{address}"));

        let config = autoscaling_test_config(true);
        let concurrency_limits = build_concurrency_limits(&config);
        concurrency_limits
            .get("/api/guest-call-legacy")
            .expect("legacy route should have a limiter")
            .pending_waiters
            .store(75, Ordering::SeqCst);
        tokio::task::spawn_blocking(move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/k8s-scaler")
                    .expect("background route should be sealed"),
                "k8s-scaler",
                telemetry::init_test_telemetry(),
                concurrency_limits,
                test_host_identity(35),
                Arc::new(StorageBrokerManager::default()),
            )
            .expect("background scaler should instantiate");

            for _ in 0..7 {
                runner.tick().expect("background tick should succeed");
            }
        })
        .await
        .expect("background runner task should complete");

        std::env::remove_var(MOCK_K8S_URL_ENV);
        server.abort();

        let requests = captured
            .lock()
            .expect("captured requests should not be poisoned");
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|body| body.contains("\"replicas\":2")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_sqs_connector_dispatches_and_acks_messages() {
        use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
        use serde_json::{json, Value};
        use std::sync::Mutex;

        #[derive(Default)]
        struct MockQueueState {
            pending: Vec<(String, String)>,
            deleted: Vec<String>,
        }

        async fn receive_messages(
            State(state): State<Arc<Mutex<MockQueueState>>>,
        ) -> impl IntoResponse {
            let state = state
                .lock()
                .expect("mock queue state should not be poisoned");
            Json(json!({
                "messages": state.pending.iter().map(|(body, receipt_handle)| json!({
                    "body": body,
                    "receipt_handle": receipt_handle,
                })).collect::<Vec<_>>()
            }))
        }

        async fn delete_message(
            State(state): State<Arc<Mutex<MockQueueState>>>,
            body: Bytes,
        ) -> StatusCode {
            let payload: Value =
                serde_json::from_slice(&body).expect("delete payload should be JSON");
            let receipt_handle = payload["receipt_handle"]
                .as_str()
                .expect("delete payload should include a receipt handle");
            let mut state = state
                .lock()
                .expect("mock queue state should not be poisoned");
            state.deleted.push(receipt_handle.to_owned());
            state
                .pending
                .retain(|(_, pending_receipt)| pending_receipt != receipt_handle);
            StatusCode::OK
        }

        let queue_state = Arc::new(Mutex::new(MockQueueState {
            pending: vec![("hello from queue".to_owned(), "receipt-1".to_owned())],
            deleted: Vec::new(),
        }));
        let queue_app = Router::new()
            .route("/queue/receive", post(receive_messages))
            .route("/queue/delete", post(delete_message))
            .with_state(Arc::clone(&queue_state));
        let queue_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock queue listener should bind");
        let queue_address = queue_listener
            .local_addr()
            .expect("mock queue listener should expose an address");
        let queue_server = tokio::spawn(async move {
            axum::serve(queue_listener, queue_app)
                .await
                .expect("mock queue server should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");
        let config = sqs_connector_test_config(
            host_address,
            format!("http://{queue_address}/queue"),
            "/api/guest-example",
            "guest-example",
        );
        let host_app = build_app(build_test_state(
            config.clone(),
            telemetry::init_test_telemetry(),
        ));
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking(move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/sqs-connector")
                    .expect("connector route should be sealed"),
                "system-faas-sqs",
                telemetry::init_test_telemetry(),
                build_concurrency_limits(&config),
                test_host_identity(36),
                Arc::new(StorageBrokerManager::default()),
            )
            .expect("SQS connector should instantiate");
            runner.tick().expect("SQS connector tick should succeed");
        })
        .await
        .expect("background connector task should complete");

        host_server.abort();
        queue_server.abort();

        let queue_state = queue_state
            .lock()
            .expect("mock queue state should not be poisoned");
        assert_eq!(queue_state.deleted, vec!["receipt-1".to_owned()]);
        assert!(queue_state.pending.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn background_sqs_connector_leaves_failed_messages_unacked() {
        use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
        use serde_json::{json, Value};
        use std::sync::Mutex;

        #[derive(Default)]
        struct MockQueueState {
            pending: Vec<(String, String)>,
            deleted: Vec<String>,
        }

        async fn receive_messages(
            State(state): State<Arc<Mutex<MockQueueState>>>,
        ) -> impl IntoResponse {
            let state = state
                .lock()
                .expect("mock queue state should not be poisoned");
            Json(json!({
                "messages": state.pending.iter().map(|(body, receipt_handle)| json!({
                    "body": body,
                    "receipt_handle": receipt_handle,
                })).collect::<Vec<_>>()
            }))
        }

        async fn delete_message(
            State(state): State<Arc<Mutex<MockQueueState>>>,
            body: Bytes,
        ) -> StatusCode {
            let payload: Value =
                serde_json::from_slice(&body).expect("delete payload should be JSON");
            let receipt_handle = payload["receipt_handle"]
                .as_str()
                .expect("delete payload should include a receipt handle");
            let mut state = state
                .lock()
                .expect("mock queue state should not be poisoned");
            state.deleted.push(receipt_handle.to_owned());
            state
                .pending
                .retain(|(_, pending_receipt)| pending_receipt != receipt_handle);
            StatusCode::OK
        }

        let queue_state = Arc::new(Mutex::new(MockQueueState {
            pending: vec![("force-fail".to_owned(), "receipt-2".to_owned())],
            deleted: Vec::new(),
        }));
        let queue_app = Router::new()
            .route("/queue/receive", post(receive_messages))
            .route("/queue/delete", post(delete_message))
            .with_state(Arc::clone(&queue_state));
        let queue_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock queue listener should bind");
        let queue_address = queue_listener
            .local_addr()
            .expect("mock queue listener should expose an address");
        let queue_server = tokio::spawn(async move {
            axum::serve(queue_listener, queue_app)
                .await
                .expect("mock queue server should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");
        let config = sqs_connector_test_config(
            host_address,
            format!("http://{queue_address}/queue"),
            "/api/connector-target",
            "guest-flaky",
        );
        let host_app = build_app(build_test_state(
            config.clone(),
            telemetry::init_test_telemetry(),
        ));
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking(move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/sqs-connector")
                    .expect("connector route should be sealed"),
                "system-faas-sqs",
                telemetry::init_test_telemetry(),
                build_concurrency_limits(&config),
                test_host_identity(37),
                Arc::new(StorageBrokerManager::default()),
            )
            .expect("SQS connector should instantiate");
            runner.tick().expect("SQS connector tick should succeed");
        })
        .await
        .expect("background connector task should complete");

        host_server.abort();
        queue_server.abort();

        let queue_state = queue_state
            .lock()
            .expect("mock queue state should not be poisoned");
        assert!(queue_state.deleted.is_empty());
        assert_eq!(queue_state.pending.len(), 1);
        assert_eq!(queue_state.pending[0].1, "receipt-2");
    }

    #[tokio::test]
    async fn router_sheds_system_routes_when_host_is_saturated() {
        let telemetry = telemetry::init_test_telemetry();
        let mut active_guards = Vec::new();
        for _ in 0..=SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD {
            active_guards.push(telemetry::begin_request(&telemetry));
        }

        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry,
        ));

        let response = app
            .oneshot(
                Request::get("/metrics")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        drop(active_guards);
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
                true
            }
        });
        let app = build_app(build_test_state(
            IntegrityConfig {
                telemetry_sample_rate: 1.0,
                ..IntegrityConfig::default_sealed()
            },
            telemetry,
        ));

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
        assert_eq!(record["sampled"], true);
        assert_eq!(record["status"], 200);
        assert!(record["trace_id"].as_str().is_some());
        assert!(record["traceparent"].as_str().is_some());
        assert!(record["fuel_consumed"].as_u64().is_some());
        assert!(record["total_duration_us"].as_u64().is_some());
        assert!(record["wasm_duration_us"].as_u64().is_some());
        assert!(record["host_overhead_us"].as_u64().is_some());
    }

    #[tokio::test]
    async fn router_skips_telemetry_export_for_unsampled_requests() {
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
                true
            }
        });
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry,
        ));

        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("Hello Lean FaaS!"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .is_empty(),
            "unsampled requests should not enqueue telemetry export records"
        );
    }

    #[tokio::test]
    async fn metering_exporter_drains_sampled_records_off_request_path() {
        use std::time::Duration;

        let metering_dir = unique_test_dir("tachyon-metering-export");
        let (export_sender, export_receiver) = mpsc::channel(TELEMETRY_EXPORT_QUEUE_CAPACITY);
        let telemetry = telemetry::init_test_telemetry_with_emitter(move |line| {
            export_sender.try_send(line).is_ok()
        });
        let config = IntegrityConfig {
            telemetry_sample_rate: 1.0,
            routes: vec![
                IntegrityRoute::user_with_secrets(DEFAULT_ROUTE, &["DB_PASS"]),
                metering_test_route(&metering_dir),
            ],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config, telemetry);
        spawn_metering_exporter(state.clone(), export_receiver);
        let app = build_app(state);

        let response = app
            .oneshot(
                Request::post("/api/guest-example")
                    .body(Body::from("Hello Lean FaaS!"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);

        let metering_file = metering_dir.join("metering.ndjson");
        let contents = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(contents) = fs::read_to_string(&metering_file) {
                    if !contents.trim().is_empty() {
                        break contents;
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("metering exporter should flush a batch");

        assert!(contents.contains("\"path\":\"/api/guest-example\""));
        assert!(contents.contains("\"sampled\":true"));
        assert!(contents.contains("\"fuel_consumed\":"));

        let _ = fs::remove_dir_all(metering_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_layer4_listener_echoes_and_releases_route_permit() {
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let port = free_tcp_port();
        let route = tcp_echo_test_route(1);
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            layer4: IntegrityLayer4Config {
                tcp: vec![IntegrityTcpBinding {
                    port,
                    target: "guest-tcp-echo".to_owned(),
                }],
                udp: Vec::new(),
            },
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        })
        .expect("TCP Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listeners = start_tcp_layer4_listeners(state.clone())
            .await
            .expect("TCP Layer 4 listener should start");
        let listener_addr = listeners
            .first()
            .expect("one TCP Layer 4 listener should be started")
            .local_addr;

        let mut stream = tokio::net::TcpStream::connect(listener_addr)
            .await
            .expect("TCP client should connect");
        stream
            .write_all(b"ping over tcp")
            .await
            .expect("TCP client should write");
        stream
            .shutdown()
            .await
            .expect("TCP client should close write");

        let mut echoed = Vec::new();
        stream
            .read_to_end(&mut echoed)
            .await
            .expect("TCP client should read echoed bytes");
        assert_eq!(echoed, b"ping over tcp");

        let runtime = state.runtime.load_full();
        let control = runtime
            .concurrency_limits
            .get(&route.path)
            .expect("TCP route should have a limiter");
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if control.semaphore.available_permits() == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP Layer 4 permit should be released after disconnect");

        for listener in listeners {
            listener.join_handle.abort();
            let _ = listener.join_handle.await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_layer4_connection_handler_echoes_payload() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let route = tcp_echo_test_route(1);
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        })
        .expect("TCP Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener should bind");
        let listener_addr = listener
            .local_addr()
            .expect("test listener should expose a local address");

        let client = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(listener_addr)
                .await
                .expect("TCP client should connect");
            stream
                .write_all(b"ping over tcp")
                .await
                .expect("TCP client should write");
            stream
                .shutdown()
                .await
                .expect("TCP client should close write");

            let mut echoed = Vec::new();
            stream
                .read_to_end(&mut echoed)
                .await
                .expect("TCP client should read echoed bytes");
            echoed
        });

        let (server_stream, _) = listener.accept().await.expect("listener should accept");
        handle_tcp_layer4_connection(state, route, server_stream)
            .await
            .expect("TCP Layer 4 connection should complete");

        assert_eq!(
            client.await.expect("client task should finish"),
            b"ping over tcp"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_layer4_listener_streams_echo_before_client_eof() {
        use std::time::Duration;

        let port = free_tcp_port();
        let route = tcp_echo_test_route(1);
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            layer4: IntegrityLayer4Config {
                tcp: vec![IntegrityTcpBinding {
                    port,
                    target: "guest-tcp-echo".to_owned(),
                }],
                udp: Vec::new(),
            },
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("TCP Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listeners = start_tcp_layer4_listeners(state)
            .await
            .expect("TCP Layer 4 listener should start");
        let listener_addr = listeners
            .first()
            .expect("one TCP Layer 4 listener should be started")
            .local_addr;

        let trailing = std::thread::spawn(move || {
            use std::io::{Read, Write};

            let mut stream =
                std::net::TcpStream::connect(listener_addr).expect("TCP client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .expect("TCP client should set a read timeout");
            stream
                .write_all(b"ping")
                .expect("TCP client should write first chunk");

            let mut first_chunk = [0_u8; 4];
            stream
                .read_exact(&mut first_chunk)
                .expect("TCP listener should echo before client EOF");
            assert_eq!(&first_chunk, b"ping");

            stream
                .write_all(b" pong")
                .expect("TCP client should write second chunk");

            let mut second_chunk = [0_u8; 5];
            stream
                .read_exact(&mut second_chunk)
                .expect("TCP listener should keep streaming echoed chunks");
            assert_eq!(&second_chunk, b" pong");

            stream
                .shutdown(std::net::Shutdown::Write)
                .expect("TCP client should close write side");

            let mut trailing = Vec::new();
            stream
                .read_to_end(&mut trailing)
                .expect("TCP client should drain trailing bytes");
            trailing
        })
        .join()
        .expect("TCP client thread should finish");
        assert!(trailing.is_empty());

        for listener in listeners {
            listener.join_handle.abort();
            let _ = listener.join_handle.await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn https_listener_provisions_mock_certificate_once_and_serves_custom_domain() {
        init_host_tracing();
        let domain = "api.example.test";
        let cert_dir = unique_test_dir("tachyon-cert-manager-http");
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.domains = vec![domain.to_owned()];
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            tls_address: Some("127.0.0.1:0".to_owned()),
            routes: vec![route, cert_manager_test_route(&cert_dir)],
            ..IntegrityConfig::default_sealed()
        })
        .expect("HTTPS config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let app = build_app(state.clone());
        let listener = start_https_listener(state.clone(), app)
            .await
            .expect("HTTPS listener should start")
            .expect("HTTPS listener should be enabled");
        let listener_addr = listener.local_addr;
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .resolve(domain, listener_addr)
            .build()
            .expect("reqwest HTTPS client should build");
        let url = format!("https://{domain}:{}/", listener_addr.port());

        let first = client
            .get(&url)
            .send()
            .await
            .expect("first HTTPS request should succeed");
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(
            first.text().await.expect("response body should decode"),
            expected_guest_example_body("FaaS received an empty payload")
        );
        assert_eq!(state.tls_manager.provision_count(), 1);
        assert!(
            cert_dir.join(format!("{domain}.json")).exists(),
            "cert-manager should persist issued material through the storage broker"
        );

        let second = client
            .get(&url)
            .send()
            .await
            .expect("cached HTTPS request should succeed");
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(state.tls_manager.provision_count(), 1);

        listener.join_handle.abort();
        let _ = listener.join_handle.await;
        let _ = fs::remove_dir_all(cert_dir);
    }

    #[cfg(feature = "http3")]
    #[tokio::test(flavor = "multi_thread")]
    async fn http3_listener_serves_guest_routes_over_quic() {
        use bytes::Buf;
        use h3::client;
        use quinn::crypto::rustls::QuicClientConfig;

        init_host_tracing();
        let domain = "api.example.test";
        let cert_dir = unique_test_dir("tachyon-cert-manager-http3");
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.domains = vec![domain.to_owned()];
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            tls_address: Some("127.0.0.1:0".to_owned()),
            routes: vec![route, cert_manager_test_route(&cert_dir)],
            ..IntegrityConfig::default_sealed()
        })
        .expect("HTTP/3 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let app = build_app(state.clone());
        let listener = start_http3_listener(state.clone(), app)
            .await
            .expect("HTTP/3 listener should start")
            .expect("HTTP/3 listener should be enabled");
        let listener_addr = listener.local_addr;

        let mut client_crypto = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("HTTP/3 client protocol versions should build")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();
        client_crypto.enable_early_data = true;
        client_crypto.alpn_protocols = vec![b"h3".to_vec()];
        let client_config = quinn::ClientConfig::new(Arc::new(
            QuicClientConfig::try_from(client_crypto).expect("HTTP/3 client config should convert"),
        ));
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap())
            .expect("HTTP/3 client endpoint should bind");
        endpoint.set_default_client_config(client_config);
        let connection = endpoint
            .connect(listener_addr, domain)
            .expect("HTTP/3 connect future should build")
            .await
            .expect("HTTP/3 handshake should succeed");

        let (mut driver, mut sender) = client::new(h3_quinn::Connection::new(connection.clone()))
            .await
            .expect("HTTP/3 client should initialize");
        let drive_task = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });
        let url = format!(
            "https://{domain}:{}/api/guest-example",
            listener_addr.port()
        );
        let mut request_stream = sender
            .send_request(
                Request::get(&url)
                    .body(())
                    .expect("HTTP/3 request should build"),
            )
            .await
            .expect("HTTP/3 request should send");
        request_stream
            .finish()
            .await
            .expect("HTTP/3 request body should finish");
        let response = request_stream
            .recv_response()
            .await
            .expect("HTTP/3 response head should arrive");
        assert_eq!(response.status(), StatusCode::OK);

        let mut body = Vec::new();
        while let Some(chunk) = request_stream
            .recv_data()
            .await
            .expect("HTTP/3 response body should stream")
        {
            let mut chunk = chunk;
            let bytes = chunk.copy_to_bytes(chunk.remaining());
            body.extend_from_slice(&bytes);
        }
        assert_eq!(
            String::from_utf8(body).expect("HTTP/3 response body should be UTF-8"),
            expected_guest_example_body("FaaS received an empty payload")
        );

        connection.close(0u32.into(), b"done");
        let _ = drive_task.await;
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
        endpoint.wait_idle().await;
        let _ = fs::remove_dir_all(cert_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tcp_layer4_listener_accepts_tls_when_route_declares_domains() {
        init_host_tracing();
        let domain = "echo.example.test";
        let cert_dir = unique_test_dir("tachyon-cert-manager-tcp");
        let port = free_tcp_port();
        let mut route = tcp_echo_test_route(1);
        route.domains = vec![domain.to_owned()];
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            layer4: IntegrityLayer4Config {
                tcp: vec![IntegrityTcpBinding {
                    port,
                    target: "guest-tcp-echo".to_owned(),
                }],
                udp: Vec::new(),
            },
            routes: vec![route, cert_manager_test_route(&cert_dir)],
            ..IntegrityConfig::default_sealed()
        })
        .expect("TLS Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listeners = start_tcp_layer4_listeners(state.clone())
            .await
            .expect("TCP listener should start");
        let listener_addr = listeners
            .first()
            .expect("one TCP listener should be started")
            .local_addr;
        let connector = insecure_tls_connector();
        let tcp_stream = tokio::net::TcpStream::connect(listener_addr)
            .await
            .expect("TLS TCP client should connect");
        let server_name =
            ServerName::try_from(domain.to_owned()).expect("server name should be valid");
        let mut tls_stream = connector
            .connect(server_name, tcp_stream)
            .await
            .expect("TLS handshake should succeed");

        tls_stream
            .write_all(b"ping over tls")
            .await
            .expect("TLS client should write");
        tls_stream
            .shutdown()
            .await
            .expect("TLS client should close write side");

        let mut echoed = Vec::new();
        tls_stream
            .read_to_end(&mut echoed)
            .await
            .expect("TLS client should read echoed bytes");
        assert_eq!(echoed, b"ping over tls");
        assert_eq!(state.tls_manager.provision_count(), 1);

        for listener in listeners {
            listener.join_handle.abort();
            let _ = listener.join_handle.await;
        }
        let _ = fs::remove_dir_all(cert_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn udp_layer4_listener_echoes_datagrams() {
        use std::time::{Duration, Instant};

        let port = free_udp_port();
        let route = udp_echo_test_route(1);
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            layer4: IntegrityLayer4Config {
                tcp: Vec::new(),
                udp: vec![IntegrityUdpBinding {
                    port,
                    target: "guest-udp-echo".to_owned(),
                }],
            },
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("UDP Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listeners = start_udp_layer4_listeners(state)
            .await
            .expect("UDP Layer 4 listener should start");
        let listener_addr = listeners
            .first()
            .expect("one UDP Layer 4 listener should be started")
            .local_addr;

        let client =
            std::net::UdpSocket::bind("127.0.0.1:0").expect("UDP client socket should bind");
        client
            .connect(listener_addr)
            .expect("UDP client should connect to listener");
        client
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("UDP client should set a read timeout");
        client
            .send(b"ping over udp")
            .expect("UDP client should send datagram");

        let started = Instant::now();
        loop {
            let mut buffer = [0_u8; 64];
            match client.recv(&mut buffer) {
                Ok(received) => {
                    assert_eq!(&buffer[..received], b"ping over udp");
                    break;
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    assert!(
                        started.elapsed() <= Duration::from_secs(10),
                        "UDP client should receive echoed datagram before timing out"
                    );
                }
                Err(error) => panic!("UDP client should receive echoed datagram: {error}"),
            }
        }

        for listener in listeners {
            for handle in listener.join_handles {
                handle.abort();
                let _ = handle.await;
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn udp_layer4_listener_drops_when_safe_queue_is_full() {
        use std::time::{Duration, Instant};

        let port = free_udp_port();
        let route = udp_echo_test_route(1);
        let config = validate_integrity_config(IntegrityConfig {
            host_address: "127.0.0.1:8080".to_owned(),
            layer4: IntegrityLayer4Config {
                tcp: Vec::new(),
                udp: vec![IntegrityUdpBinding {
                    port,
                    target: "guest-udp-echo".to_owned(),
                }],
            },
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("UDP Layer 4 config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let listeners = start_udp_layer4_listeners_with_queue_capacity(state, 1)
            .await
            .expect("UDP Layer 4 listener should start");
        let listener_addr = listeners
            .first()
            .expect("one UDP Layer 4 listener should be started")
            .local_addr;

        let client =
            std::net::UdpSocket::bind("127.0.0.1:0").expect("UDP client socket should bind");
        client
            .connect(listener_addr)
            .expect("UDP client should connect to listener");
        client
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("UDP client should set a read timeout");

        client
            .send(b"delay:200")
            .expect("UDP client should send slow datagram");
        for index in 0..16 {
            let payload = format!("packet-{index}");
            client
                .send(payload.as_bytes())
                .expect("UDP client should send queued datagram");
        }

        let started = Instant::now();
        let mut responses = Vec::new();
        while started.elapsed() <= Duration::from_secs(10) {
            let mut buffer = [0_u8; 64];
            match client.recv(&mut buffer) {
                Ok(received) => {
                    responses.push(String::from_utf8_lossy(&buffer[..received]).into_owned())
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        || error.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(error) => panic!("UDP client receive should not fail: {error}"),
            }
        }

        assert!(
            responses.iter().any(|payload| payload == "delay:200"),
            "the initially accepted datagram should complete"
        );
        assert!(
            responses.len() <= 2,
            "queue overload should drop excess datagrams, got {responses:?}"
        );

        for listener in listeners {
            for handle in listener.join_handles {
                handle.abort();
                let _ = handle.await;
            }
        }
    }

    #[tokio::test]
    async fn websocket_upgrade_is_rejected_without_feature_flag() {
        let route = targeted_route("/ws/echo", vec![websocket_target("guest-websocket-echo")]);
        let app = build_app(build_test_state(
            IntegrityConfig {
                routes: vec![route],
                ..IntegrityConfig::default_sealed()
            },
            telemetry::init_test_telemetry(),
        ));

        let response = app
            .oneshot(
                Request::get("/ws/echo")
                    .header("connection", "Upgrade")
                    .header("upgrade", "websocket")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[cfg(feature = "websockets")]
    #[tokio::test(flavor = "multi_thread")]
    async fn websocket_route_upgrades_and_echoes_frames() {
        use futures_util::{SinkExt, StreamExt};
        use std::time::Duration;
        use tokio_tungstenite::tungstenite::Message;

        let route = targeted_route("/ws/echo", vec![websocket_target("guest-websocket-echo")]);
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("WebSocket route config should validate");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("WebSocket test listener should bind");
        let address = listener
            .local_addr()
            .expect("WebSocket test listener should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .expect("WebSocket test server should stay up");
        });

        let url = format!("ws://{address}/ws/echo");
        let (mut client, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("WebSocket client should connect");

        client
            .send(Message::Text("hello".into()))
            .await
            .expect("WebSocket client should send text frame");
        let text_frame = client
            .next()
            .await
            .expect("WebSocket server should respond")
            .expect("WebSocket frame should be valid");
        assert!(matches!(text_frame, Message::Text(text) if text == "hello"));

        client
            .send(Message::Binary(vec![1_u8, 2, 3]))
            .await
            .expect("WebSocket client should send binary frame");
        let binary_frame = client
            .next()
            .await
            .expect("WebSocket server should respond to binary frame")
            .expect("WebSocket frame should be valid");
        assert!(matches!(binary_frame, Message::Binary(bytes) if bytes == vec![1_u8, 2, 3]));

        client
            .close(None)
            .await
            .expect("WebSocket client should initiate close");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match client.next().await {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => panic!("WebSocket close should not error: {error}"),
                }
            }
        })
        .await
        .expect("WebSocket guest should shut down after close");

        server.abort();
        let _ = server.await;
    }

    #[cfg(feature = "secrets-vault")]
    #[tokio::test]
    async fn router_denies_secret_lookup_without_sealed_grant() {
        let app = build_app(build_test_state(
            IntegrityConfig {
                routes: vec![IntegrityRoute::user("/api/guest-example")],
                ..IntegrityConfig::default_sealed()
            },
            telemetry::init_test_telemetry(),
        ));
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
            "FaaS received an empty payload | env: missing | secret: permission-denied"
        );
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
    fn select_route_module_prefers_matching_header_targets() {
        let route = targeted_route(
            "/api/checkout",
            vec![
                header_target("guest-loop", COHORT_HEADER, "beta"),
                weighted_target("guest-example", 100),
            ],
        );
        let mut headers = HeaderMap::new();
        headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));

        assert_eq!(
            select_route_target_with_roll(&route, &headers, Some(42))
                .expect("header-target route should resolve"),
            SelectedRouteTarget {
                module: "guest-loop".to_owned(),
                websocket: false,
            }
        );
    }

    #[test]
    fn select_route_module_uses_weighted_rollout_without_matching_headers() {
        let route = targeted_route(
            "/api/checkout",
            vec![
                weighted_target("guest-example", 90),
                weighted_target("guest-loop", 10),
            ],
        );

        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("weighted route should resolve"),
            SelectedRouteTarget {
                module: "guest-example".to_owned(),
                websocket: false,
            }
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(89))
                .expect("weighted route should resolve"),
            SelectedRouteTarget {
                module: "guest-example".to_owned(),
                websocket: false,
            }
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(90))
                .expect("weighted route should resolve"),
            SelectedRouteTarget {
                module: "guest-loop".to_owned(),
                websocket: false,
            }
        );
    }

    #[test]
    fn select_route_module_falls_back_to_path_module_when_targets_are_header_only() {
        let route = targeted_route(
            "/api/guest-example",
            vec![header_target("guest-loop", COHORT_HEADER, "beta")],
        );

        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("route should fall back to the path module"),
            SelectedRouteTarget {
                module: "guest-example".to_owned(),
                websocket: false,
            }
        );
    }

    #[test]
    fn extract_propagated_headers_copies_legacy_and_canonical_cohort_names() {
        let mut headers = HeaderMap::new();
        headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));

        assert_eq!(
            extract_propagated_headers(&headers),
            vec![
                PropagatedHeader {
                    name: COHORT_HEADER.to_owned(),
                    value: "beta".to_owned(),
                },
                PropagatedHeader {
                    name: TACHYON_COHORT_HEADER.to_owned(),
                    value: "beta".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn resolve_incoming_hop_limit_defaults_missing_or_invalid_values() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_incoming_hop_limit(&headers),
            Ok(HopLimit(DEFAULT_HOP_LIMIT))
        );

        let mut headers = HeaderMap::new();
        headers.insert(HOP_LIMIT_HEADER, HeaderValue::from_static("not-a-number"));
        assert_eq!(
            resolve_incoming_hop_limit(&headers),
            Ok(HopLimit(DEFAULT_HOP_LIMIT))
        );
    }

    #[test]
    fn resolve_incoming_hop_limit_rejects_zero() {
        let mut headers = HeaderMap::new();
        headers.insert(HOP_LIMIT_HEADER, HeaderValue::from_static("0"));

        assert_eq!(resolve_incoming_hop_limit(&headers), Err(()));
    }

    #[test]
    fn resolve_mesh_fetch_target_supports_relative_mesh_routes() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should stay sealed");

        assert_eq!(
            resolve_mesh_fetch_target(&config, &route_registry, caller_route, "/api/guest-loop",)
                .expect("relative mesh route should resolve"),
            "http://127.0.0.1:8080/api/guest-loop"
        );
    }

    #[test]
    fn resolve_mesh_fetch_target_uses_highest_compatible_dependency_version() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![
            dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]),
            versioned_route("/api/faas-b-v2", "faas-b", "2.1.0"),
            versioned_route("/api/faas-b-v3", "faas-b", "3.0.0"),
        ];
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/faas-a")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_mesh_fetch_target(
                &config,
                &route_registry,
                caller_route,
                "http://tachyon/faas-b",
            )
            .expect("dependency route should resolve"),
            "http://127.0.0.1:8080/api/faas-b-v2"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_forwards_propagated_cohort_headers() {
        use axum::{extract::State, routing::get, Router};

        async fn capture_headers(
            State(captured): State<CapturedForwardedHeaders>,
            headers: HeaderMap,
        ) -> &'static str {
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .push((
                    headers
                        .get(HOP_LIMIT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(COHORT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(TACHYON_COHORT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(TACHYON_IDENTITY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                ));
            "ok"
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/ping", get(capture_headers))
            .with_state(Arc::clone(&captured));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener
            .local_addr()
            .expect("mock server should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should stay healthy");
        });

        let mut inbound_headers = HeaderMap::new();
        inbound_headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));
        inbound_headers.insert(
            TACHYON_IDENTITY_HEADER,
            HeaderValue::from_static("Bearer spoofed"),
        );
        let propagated_headers = extract_propagated_headers(&inbound_headers);
        let host_identity = test_host_identity(40);
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
            &new_uds_fast_path_registry(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &propagated_headers,
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("mesh fetch should succeed");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("ok"));
        let captured = captured
            .lock()
            .expect("captured headers should not be poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, (DEFAULT_HOP_LIMIT - 1).to_string());
        assert_eq!(captured[0].1, "beta");
        assert_eq!(captured[0].2, "beta");
        assert_ne!(captured[0].3, "Bearer spoofed");
        let claims = host_identity
            .verify_token(
                captured[0]
                    .3
                    .strip_prefix("Bearer ")
                    .expect("mesh identity header should include a bearer token"),
            )
            .expect("mesh identity header should verify");
        assert_eq!(claims.route_path, DEFAULT_ROUTE);
        assert_eq!(claims.role, RouteRole::User);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_does_not_leak_identity_headers_to_external_targets() {
        use axum::{extract::State, routing::get, Router};

        async fn capture_identity_header(
            State(captured): State<Arc<std::sync::Mutex<Vec<String>>>>,
            headers: HeaderMap,
        ) -> &'static str {
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .push(
                    headers
                        .get(TACHYON_IDENTITY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                );
            "ok"
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/ping", get(capture_identity_header))
            .with_state(Arc::clone(&captured));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener
            .local_addr()
            .expect("mock server should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should stay healthy");
        });

        let config = IntegrityConfig::default_sealed();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let host_identity = test_host_identity(41);
        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
            &new_uds_fast_path_registry(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, format!("MESH_FETCH:http://{address}/ping")),
        )
        .await
        .expect("external mesh fetch should succeed");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("ok"));
        assert_eq!(
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .as_slice(),
            &["".to_owned()]
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn uds_fast_path_registration_publishes_socket_metadata() {
        let discovery_dir = unique_test_dir("tachyon-uds-discovery");
        let registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
            discovery_dir.clone(),
        ));
        let config = IntegrityConfig {
            host_address: "127.0.0.1:19090".to_owned(),
            ..IntegrityConfig::default_sealed()
        };
        let app = axum::Router::new().route("/ping", axum::routing::get(|| async { "ok" }));
        let server = start_uds_fast_path_listener(app, &config, Arc::clone(&registry))
            .expect("UDS listener should register")
            .expect("UDS listener should start on Unix");

        tokio::time::sleep(Duration::from_millis(50)).await;

        let metadata_files = fs::read_dir(&discovery_dir)
            .expect("discovery dir should exist")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        assert_eq!(metadata_files.len(), 1);

        let metadata: UdsPeerMetadata = serde_json::from_slice(
            &fs::read(&metadata_files[0]).expect("metadata should be readable"),
        )
        .expect("metadata should parse");
        assert_eq!(metadata.ip, "127.0.0.1");
        assert!(
            Path::new(&metadata.socket_path).exists(),
            "published UDS socket should exist"
        );

        server.abort();
        let _ = server.await;
        drop(registry);
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_prefers_local_uds_fast_path() {
        use axum::routing::get;

        let discovery_dir = unique_test_dir("tachyon-uds-fast-path");
        let registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
            discovery_dir.clone(),
        ));
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "127.0.0.1:19191".to_owned();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let app = axum::Router::new().route("/ping", get(|| async { "uds-fast-path" }));
        let server = start_uds_fast_path_listener(app, &config, Arc::clone(&registry))
            .expect("UDS listener should register")
            .expect("UDS listener should start on Unix");

        tokio::time::sleep(Duration::from_millis(50)).await;

        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            test_host_identity(42).as_ref(),
            registry.as_ref(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("UDS fast-path request should succeed");

        server.abort();
        let _ = server.await;
        drop(registry);

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("uds-fast-path"));
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_falls_back_to_tcp_when_peer_metadata_is_stale() {
        use axum::routing::get;

        let discovery_dir = unique_test_dir("tachyon-uds-stale-peer");
        let metadata_path = discovery_dir.join("stale-peer.json");
        fs::create_dir_all(&discovery_dir).expect("discovery dir should be created");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP listener should bind");
        let address = listener
            .local_addr()
            .expect("TCP listener should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/ping", get(|| async { "tcp-fallback" })),
            )
            .await
            .expect("TCP fallback server should stay healthy");
        });

        let stale_socket = discovery_dir.join("missing.sock");
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&UdsPeerMetadata {
                host_id: "stale".to_owned(),
                ip: "127.0.0.1".to_owned(),
                socket_path: stale_socket.display().to_string(),
                protocols: vec!["http/1.1".to_owned()],
                pressure_state: PeerPressureState::Idle,
                last_pressure_update_unix_ms: 0,
            })
            .expect("stale metadata should serialize"),
        )
        .expect("stale metadata should be written");

        let registry = UdsFastPathRegistry::with_discovery_dir(discovery_dir.clone());
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");

        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            test_host_identity(43).as_ref(),
            &registry,
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("stale peer should fall back to TCP");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("tcp-fallback"));
        assert!(
            !metadata_path.exists(),
            "missing-socket metadata should be removed during discovery refresh"
        );
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "manual latency benchmark for UDS fast-path validation"]
    async fn uds_fast_path_is_faster_than_loopback_tcp_for_repeated_mesh_fetches() {
        use axum::routing::get;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP listener should bind");
        let address = listener
            .local_addr()
            .expect("TCP listener should expose an address");
        let tcp_server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/ping", get(|| async { "ok" })),
            )
            .await
            .expect("TCP benchmark server should stay healthy");
        });

        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let host_identity = test_host_identity(44);

        let discovery_dir = unique_test_dir("tachyon-uds-benchmark");
        let uds_registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
            discovery_dir.clone(),
        ));
        let uds_server = start_uds_fast_path_listener(
            axum::Router::new().route("/ping", get(|| async { "ok" })),
            &config,
            Arc::clone(&uds_registry),
        )
        .expect("UDS benchmark server should register")
        .expect("UDS benchmark server should start");
        tokio::time::sleep(Duration::from_millis(50)).await;

        async fn benchmark(
            registry: &UdsFastPathRegistry,
            config: &IntegrityConfig,
            route_registry: &RouteRegistry,
            caller_route: &IntegrityRoute,
            host_identity: &HostIdentity,
        ) -> Duration {
            let start = Instant::now();
            for _ in 0..24 {
                let response = resolve_mesh_response(
                    &Client::new(),
                    config,
                    route_registry,
                    caller_route,
                    host_identity,
                    registry,
                    HopLimit(DEFAULT_HOP_LIMIT),
                    &[],
                    GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
                )
                .await
                .expect("benchmark mesh fetch should succeed");
                assert_eq!(response.status, StatusCode::OK);
            }
            start.elapsed()
        }

        let tcp_registry = new_uds_fast_path_registry();
        let tcp_elapsed = benchmark(
            &tcp_registry,
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
        )
        .await;
        let uds_elapsed = benchmark(
            uds_registry.as_ref(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
        )
        .await;

        uds_server.abort();
        let _ = uds_server.await;
        tcp_server.abort();

        assert!(
            uds_elapsed < tcp_elapsed,
            "UDS fast-path should beat loopback TCP (uds={uds_elapsed:?}, tcp={tcp_elapsed:?})"
        );
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[tokio::test]
    async fn router_breaks_guest_self_loop_with_http_508() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose an address");

        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        config.routes.push(IntegrityRoute::user("/api/guest-loop"));

        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("test host should stay healthy");
        });

        let response = Client::new()
            .get(format!("http://{address}/api/guest-loop"))
            .send()
            .await
            .expect("guest-loop request should complete");

        let status = response.status();
        let body = response
            .text()
            .await
            .expect("guest-loop response body should be readable");

        let _ = shutdown_tx.send(());
        server.await.expect("server should shut down cleanly");

        assert_eq!(status, StatusCode::LOOP_DETECTED);
        assert!(
            body.contains("Routing loop detected"),
            "unexpected loop-detected response body: {body}"
        );
    }

    #[tokio::test]
    async fn graceful_shutdown_waits_for_in_flight_requests() {
        use axum::routing::get;
        use tokio::sync::Notify;

        async fn slow_handler(State(started): State<Arc<Notify>>) -> &'static str {
            started.notify_one();
            tokio::time::sleep(Duration::from_millis(150)).await;
            "done"
        }

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose an address");
        let started = Arc::new(Notify::new());
        let app = Router::new()
            .route("/slow", get(slow_handler))
            .with_state(Arc::clone(&started));

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("server should shut down cleanly");
        });

        let request = tokio::spawn(async move {
            Client::new()
                .get(format!("http://{address}/slow"))
                .send()
                .await
                .expect("request should complete")
        });

        started.notified().await;
        let _ = shutdown_tx.send(());

        let response = request.await.expect("request task should complete");
        let status = response.status();
        let body = response
            .text()
            .await
            .expect("response body should be readable");

        server.await.expect("server task should complete");

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "done");
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
            IntegrityRoute::user("api/guest-example"),
            IntegrityRoute::user("/api/guest-malicious"),
            IntegrityRoute::system("/metrics/"),
        ];

        let config = validate_integrity_config(config).expect("config should validate");

        assert_eq!(
            config.routes,
            vec![
                IntegrityRoute::user("/api/guest-example"),
                IntegrityRoute::user("/api/guest-malicious"),
                IntegrityRoute::system("/metrics"),
            ]
        );
    }

    #[test]
    fn validate_integrity_config_accepts_batch_targets_without_routes() {
        let temp_dir = unique_test_dir("batch-targets");
        let cache_dir = temp_dir.join("cache");
        fs::create_dir_all(&cache_dir).expect("cache directory should be created");

        let mut config = IntegrityConfig::default_sealed();
        config.routes.clear();
        config.batch_targets = vec![gc_batch_target(&cache_dir, 60)];

        let config = validate_integrity_config(config).expect("batch-only config should validate");

        assert!(config.routes.is_empty());
        assert_eq!(config.batch_targets.len(), 1);
        assert_eq!(config.batch_targets[0].name, "gc-job");
    }

    #[test]
    fn validate_integrity_config_rejects_duplicate_routes() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![
            IntegrityRoute::user("/metrics"),
            IntegrityRoute::system("/metrics/"),
        ];

        let error = validate_integrity_config(config)
            .expect_err("duplicate normalized routes should fail validation");

        assert!(error.to_string().contains("defined more than once"));
    }

    #[test]
    fn validate_integrity_config_defaults_route_scaling_for_older_payloads() {
        let config = serde_json::from_str::<IntegrityConfig>(
            r#"{
                "host_address":"0.0.0.0:8080",
                "max_stdout_bytes":65536,
                "guest_fuel_budget":500000000,
                "guest_memory_limit_bytes":52428800,
                "resource_limit_response":"Execution trapped: Resource limit exceeded",
                "routes":[{"path":"/api/guest-example","role":"user"}]
            }"#,
        )
        .expect("legacy payload should deserialize");
        let config = validate_integrity_config(config).expect("legacy payload should validate");
        let route = config
            .sealed_route("/api/guest-example")
            .expect("route should remain sealed");

        assert_eq!(route.name, "guest-example");
        assert_eq!(route.version, DEFAULT_ROUTE_VERSION);
        assert!(route.dependencies.is_empty());
        assert!(route.requires_credentials.is_empty());
        assert!(route.middleware.is_none());
        assert_eq!(route.min_instances, 0);
        assert_eq!(route.max_concurrency, DEFAULT_ROUTE_MAX_CONCURRENCY);
        assert!(route.volumes.is_empty());
    }

    #[test]
    fn build_test_state_prewarms_min_instances() {
        let mut route = IntegrityRoute::user(DEFAULT_ROUTE);
        route.min_instances = 2;
        route.max_concurrency = 4;

        let state = build_test_state(
            IntegrityConfig {
                routes: vec![route.clone()],
                ..IntegrityConfig::default_sealed()
            },
            telemetry::init_test_telemetry(),
        );
        let runtime = state.runtime.load_full();
        let control = runtime
            .concurrency_limits
            .get(&route.path)
            .expect("route should have an execution control");

        assert_eq!(control.prewarmed_instances(), 2);
    }

    #[test]
    fn validate_integrity_config_rejects_invalid_telemetry_sample_rate() {
        let error = validate_integrity_config(IntegrityConfig {
            telemetry_sample_rate: 1.5,
            ..IntegrityConfig::default_sealed()
        })
        .expect_err("sample rates above one should fail validation");

        assert!(error.to_string().contains("`telemetry_sample_rate`"));
    }

    #[test]
    fn validate_integrity_config_rejects_duplicate_tcp_layer4_ports() {
        let error = validate_integrity_config(IntegrityConfig {
            layer4: IntegrityLayer4Config {
                tcp: vec![
                    IntegrityTcpBinding {
                        port: 2222,
                        target: "guest-tcp-echo".to_owned(),
                    },
                    IntegrityTcpBinding {
                        port: 2222,
                        target: "guest-tcp-echo".to_owned(),
                    },
                ],
                udp: Vec::new(),
            },
            routes: vec![tcp_echo_test_route(1)],
            ..IntegrityConfig::default_sealed()
        })
        .expect_err("duplicate TCP Layer 4 ports should fail validation");

        assert!(error.to_string().contains("Layer 4 port `2222`"));
    }

    #[test]
    fn validate_integrity_config_rejects_duplicate_udp_layer4_ports() {
        let error = validate_integrity_config(IntegrityConfig {
            layer4: IntegrityLayer4Config {
                tcp: Vec::new(),
                udp: vec![
                    IntegrityUdpBinding {
                        port: 5353,
                        target: "guest-udp-echo".to_owned(),
                    },
                    IntegrityUdpBinding {
                        port: 5353,
                        target: "guest-udp-echo".to_owned(),
                    },
                ],
            },
            routes: vec![udp_echo_test_route(1)],
            ..IntegrityConfig::default_sealed()
        })
        .expect_err("duplicate UDP Layer 4 ports should fail validation");

        assert!(error.to_string().contains("Layer 4 port `5353`"));
    }

    #[test]
    fn validate_integrity_config_rejects_unsatisfied_semver_dependencies() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![
            dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]),
            versioned_route("/api/faas-b-v1", "faas-b", "1.5.0"),
        ];

        let error = validate_integrity_config(config)
            .expect_err("unsatisfied dependency graph should fail validation");

        assert!(error.to_string().contains("requires faas-b matching ^2.0"));
    }

    #[test]
    fn validate_integrity_config_accepts_system_connector_dependencies() {
        let host_address = DEFAULT_HOST_ADDRESS
            .parse::<SocketAddr>()
            .expect("default host address should parse");
        let config = validate_integrity_config(sqs_connector_test_config(
            host_address,
            "http://queue.local/mock".to_owned(),
            "/api/connector-target",
            "guest-example",
        ))
        .expect("system connector dependencies should validate");

        let connector = config
            .sealed_route("/system/sqs-connector")
            .expect("connector route should remain sealed");
        let expected_requirement = VersionReq::parse(&default_route_version())
            .expect("default route version should normalize as a requirement")
            .to_string();

        assert_eq!(
            connector.dependencies.get("connector-target"),
            Some(&expected_requirement)
        );
        assert_eq!(
            connector.env.get("QUEUE_URL"),
            Some(&"http://queue.local/mock".to_owned())
        );
        assert_eq!(
            connector.env.get("TARGET_ROUTE"),
            Some(&"/api/connector-target".to_owned())
        );
    }

    #[test]
    fn validate_integrity_config_rejects_missing_delegated_credentials() {
        let mut config = IntegrityConfig::default_sealed();
        let faas_a = dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]);
        let mut faas_b = versioned_route("/api/faas-b-v2", "faas-b", "2.1.0");
        faas_b.requires_credentials = vec!["c2".to_owned()];
        config.routes = vec![faas_a, faas_b];

        let error = validate_integrity_config(config)
            .expect_err("missing delegated credentials should fail validation");

        assert!(error.to_string().contains("Credential delegation failed"));
        assert!(error.to_string().contains("c2"));
    }

    #[test]
    fn validate_integrity_config_accepts_satisfied_delegated_credentials() {
        let mut config = IntegrityConfig::default_sealed();
        let mut faas_a = dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]);
        faas_a.requires_credentials = vec!["c2".to_owned()];
        let mut faas_b = versioned_route("/api/faas-b-v2", "faas-b", "2.1.0");
        faas_b.requires_credentials = vec!["c2".to_owned()];
        config.routes = vec![faas_a, faas_b];

        let config = validate_integrity_config(config)
            .expect("delegated credentials should satisfy dependency validation");
        let route = config
            .sealed_route("/api/faas-a")
            .expect("caller route should remain sealed");

        assert_eq!(route.requires_credentials, vec!["c2".to_owned()]);
    }

    #[test]
    fn validate_integrity_config_rejects_unknown_middleware_route() {
        let mut config = IntegrityConfig::default_sealed();
        let mut protected = IntegrityRoute::user(DEFAULT_ROUTE);
        protected.middleware = Some("missing-auth".to_owned());
        config.routes = vec![protected];

        let error = validate_integrity_config(config)
            .expect_err("unknown middleware route should fail validation");

        assert!(error
            .to_string()
            .contains("route middleware `missing-auth`"));
    }

    #[test]
    fn middleware_routes_short_circuit_non_ok_responses_and_allow_ok_responses() {
        fn simulate_middleware_chain(
            runtime: &RuntimeState,
            route: &IntegrityRoute,
            responses: &HashMap<String, GuestHttpResponse>,
            visited: &mut Vec<String>,
        ) -> GuestHttpResponse {
            if let Some(middleware_name) = route.middleware.as_deref() {
                let middleware = runtime
                    .route_registry
                    .resolve_named_route(middleware_name)
                    .expect("middleware route should resolve");
                let middleware_route = runtime
                    .config
                    .sealed_route(&middleware.path)
                    .expect("middleware route should stay sealed");
                visited.push(middleware_route.path.clone());
                let middleware_response = responses
                    .get(&middleware_route.path)
                    .expect("middleware response should be defined")
                    .clone();
                if middleware_response.status != StatusCode::OK {
                    return middleware_response;
                }
            }

            visited.push(route.path.clone());
            responses
                .get(&route.path)
                .expect("main route response should be defined")
                .clone()
        }

        let mut protected_allow = targeted_route(
            "/api/protected-allow",
            vec![weighted_target("guest-example", 100)],
        );
        protected_allow.name = "protected-allow".to_owned();
        protected_allow.middleware = Some("allow-middleware".to_owned());

        let mut protected_deny = targeted_route(
            "/api/protected-deny",
            vec![weighted_target("guest-example", 100)],
        );
        protected_deny.name = "protected-deny".to_owned();
        protected_deny.middleware = Some("deny-middleware".to_owned());

        let mut allow_middleware = IntegrityRoute::user("/api/allow-middleware");
        allow_middleware.name = "allow-middleware".to_owned();

        let mut deny_middleware = IntegrityRoute::user("/api/deny-middleware");
        deny_middleware.name = "deny-middleware".to_owned();

        let config = IntegrityConfig {
            routes: vec![
                protected_allow.clone(),
                protected_deny.clone(),
                allow_middleware,
                deny_middleware,
            ],
            ..IntegrityConfig::default_sealed()
        };
        let runtime = build_test_runtime(
            validate_integrity_config(config).expect("test config should validate"),
        );

        let allow_route = runtime
            .config
            .sealed_route("/api/protected-allow")
            .expect("allow route should stay sealed");
        let deny_route = runtime
            .config
            .sealed_route("/api/protected-deny")
            .expect("deny route should stay sealed");

        let mut responses = HashMap::new();
        responses.insert(
            "/api/allow-middleware".to_owned(),
            GuestHttpResponse::new(StatusCode::OK, "middleware allowed"),
        );
        responses.insert(
            "/api/protected-allow".to_owned(),
            GuestHttpResponse::new(
                StatusCode::OK,
                Bytes::from(expected_guest_example_body(
                    "FaaS received an empty payload",
                )),
            ),
        );
        responses.insert(
            "/api/deny-middleware".to_owned(),
            GuestHttpResponse::new(StatusCode::FORBIDDEN, "forbidden"),
        );
        responses.insert(
            "/api/protected-deny".to_owned(),
            GuestHttpResponse::new(StatusCode::OK, "main route should not execute"),
        );

        let mut allow_visited = Vec::new();
        let allow_response =
            simulate_middleware_chain(&runtime, allow_route, &responses, &mut allow_visited);
        assert_eq!(
            allow_visited,
            vec![
                "/api/allow-middleware".to_owned(),
                "/api/protected-allow".to_owned()
            ]
        );
        assert_eq!(allow_response.status, StatusCode::OK);
        assert_eq!(
            allow_response.body,
            Bytes::from(expected_guest_example_body(
                "FaaS received an empty payload"
            ))
        );

        let mut deny_visited = Vec::new();
        let deny_response =
            simulate_middleware_chain(&runtime, deny_route, &responses, &mut deny_visited);
        assert_eq!(deny_visited, vec!["/api/deny-middleware".to_owned()]);
        assert_eq!(deny_response.status, StatusCode::FORBIDDEN);
        assert_eq!(deny_response.body, Bytes::from("forbidden"));
    }

    #[test]
    fn validate_integrity_config_normalizes_route_volumes() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: "  /tmp/tachyon_data  ".to_owned(),
                guest_path: "/app/data/".to_owned(),
                readonly: true,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }];

        let config = validate_integrity_config(config).expect("volume config should validate");
        let route = config
            .sealed_route("/api/guest-volume")
            .expect("route should remain sealed");

        assert_eq!(
            route.volumes,
            vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: "/tmp/tachyon_data".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: true,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }]
        );
    }

    #[test]
    fn validate_integrity_config_rejects_writable_user_route_volumes() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: "/tmp/tachyon_data".to_owned(),
                guest_path: "/app/data".to_owned(),
                readonly: false,
                ttl_seconds: None,
                idle_timeout: None,
                eviction_policy: None,
            }],
        }];

        let error = validate_integrity_config(config)
            .expect_err("writable user volumes should fail validation");

        assert!(error
            .to_string()
            .contains("cannot request writable direct host mounts"));
    }

    #[test]
    fn validate_integrity_config_normalizes_model_bindings() {
        let mut config = IntegrityConfig::default_sealed();
        let mut route = IntegrityRoute::user("/api/guest-ai");
        route.models = vec![IntegrityModelBinding {
            alias: " llama3 ".to_owned(),
            path: "  /models/llama3.gguf ".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::Standard,
        }];
        config.routes = vec![route];

        let config = validate_integrity_config(config).expect("model bindings should validate");
        let route = config
            .sealed_route("/api/guest-ai")
            .expect("AI route should stay available");

        assert_eq!(
            route.models,
            vec![IntegrityModelBinding {
                alias: "llama3".to_owned(),
                path: "/models/llama3.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Standard,
            }]
        );
    }

    #[test]
    fn validate_integrity_config_rejects_duplicate_model_aliases_across_routes() {
        let mut first = IntegrityRoute::user("/api/guest-ai");
        first.models = vec![IntegrityModelBinding {
            alias: "shared".to_owned(),
            path: "/models/shared-a.gguf".to_owned(),
            device: ModelDevice::Cpu,
            qos: RouteQos::Standard,
        }];
        let mut second = IntegrityRoute::user("/api/assistant");
        second.models = vec![IntegrityModelBinding {
            alias: "shared".to_owned(),
            path: "/models/shared-b.gguf".to_owned(),
            device: ModelDevice::Metal,
            qos: RouteQos::Standard,
        }];

        let error = validate_integrity_config(IntegrityConfig {
            routes: vec![first, second],
            ..IntegrityConfig::default_sealed()
        })
        .expect_err("duplicate model aliases should fail validation");

        assert!(error.to_string().contains("model alias `shared`"));
    }

    #[test]
    fn validate_integrity_config_normalizes_custom_domains() {
        let mut config = IntegrityConfig::default_sealed();
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.domains = vec![" API.Example.Test ".to_owned()];
        config.routes = vec![route];
        config.tls_address = Some(DEFAULT_TLS_ADDRESS.to_owned());

        let config = validate_integrity_config(config).expect("TLS domains should validate");
        let route = config
            .sealed_route("/api/guest-example")
            .expect("route should stay sealed");

        assert_eq!(route.domains, vec!["api.example.test".to_owned()]);
        assert_eq!(config.tls_address.as_deref(), Some(DEFAULT_TLS_ADDRESS));
    }

    #[test]
    fn validate_integrity_config_rejects_duplicate_custom_domains() {
        let mut first = IntegrityRoute::user("/api/guest-example");
        first.domains = vec!["api.example.test".to_owned()];
        let mut second = IntegrityRoute::user("/api/guest-loop");
        second.domains = vec!["api.example.test".to_owned()];

        let error = validate_integrity_config(IntegrityConfig {
            tls_address: Some(DEFAULT_TLS_ADDRESS.to_owned()),
            routes: vec![first, second],
            ..IntegrityConfig::default_sealed()
        })
        .expect_err("duplicate domains should fail validation");

        assert!(error.to_string().contains("domain `api.example.test`"));
    }

    #[test]
    fn validate_integrity_config_accepts_hibernating_ram_volume() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![hibernating_ram_route(Path::new("/tmp/tachyon-ram-cache"))];

        let config =
            validate_integrity_config(config).expect("hibernating RAM volume should validate");
        let route = config
            .sealed_route("/api/guest-volume")
            .expect("route should remain sealed");

        assert_eq!(route.volumes[0].volume_type, VolumeType::Ram);
        assert_eq!(route.volumes[0].idle_timeout.as_deref(), Some("50ms"));
        assert_eq!(
            route.volumes[0].eviction_policy,
            Some(VolumeEvictionPolicy::Hibernate)
        );
    }

    #[test]
    fn validate_integrity_config_accepts_volume_ttl_seconds() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![ttl_managed_volume_route(Path::new("/tmp/tachyon-ttl"), 300)];

        let config = validate_integrity_config(config).expect("volume ttl should validate");
        let route = config
            .sealed_route("/api/guest-volume")
            .expect("route should remain sealed");

        assert_eq!(route.volumes[0].ttl_seconds, Some(300));
    }

    #[test]
    fn collect_ttl_managed_paths_deduplicates_by_shortest_ttl() {
        let shared_dir = Path::new("/tmp/tachyon-ttl-shared");
        let config = IntegrityConfig {
            routes: vec![
                ttl_managed_volume_route(shared_dir, 300),
                ttl_managed_volume_route(shared_dir, 60),
            ],
            ..IntegrityConfig::default_sealed()
        };

        assert_eq!(
            collect_ttl_managed_paths(&config),
            vec![TtlManagedPath {
                host_path: PathBuf::from("/tmp/tachyon-ttl-shared"),
                ttl: Duration::from_secs(60),
            }]
        );
    }

    #[test]
    fn storage_broker_serializes_concurrent_writes_against_shared_volume() {
        let volume_dir = unique_test_dir("tachyon-storage-broker");
        let route = storage_broker_test_route(&volume_dir);
        let broker = StorageBrokerManager::default();
        let start = Arc::new(std::sync::Barrier::new(9));

        let handles = (0..8)
            .map(|index| {
                let broker = broker.clone();
                let route = route.clone();
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    broker
                        .enqueue_write_for_route(
                            &route,
                            "/app/data/state.txt",
                            StorageWriteMode::Append,
                            format!("write-{index}\n").into_bytes(),
                        )
                        .expect("broker write should be accepted");
                })
            })
            .collect::<Vec<_>>();

        start.wait();
        for handle in handles {
            handle.join().expect("broker worker thread should complete");
        }

        assert!(
            broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
            "broker queue should drain"
        );

        let contents = fs::read_to_string(volume_dir.join("state.txt"))
            .expect("brokered writes should reach the shared host volume");
        let mut lines = contents.lines().collect::<Vec<_>>();
        lines.sort_unstable();

        assert_eq!(
            lines,
            vec![
                "write-0", "write-1", "write-2", "write-3", "write-4", "write-5", "write-6",
                "write-7",
            ]
        );

        let _ = fs::remove_dir_all(volume_dir);
    }

    #[tokio::test]
    async fn storage_broker_enforces_signed_caller_scope_with_http_403() {
        let shared_dir = unique_test_dir("tachyon-zero-trust-broker");
        let tenant_a_dir = shared_dir.join("tenant-a");
        let tenant_b_dir = shared_dir.join("tenant-b");
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![
                scoped_volume_test_route("/api/tenant-a", &tenant_a_dir, "/data/tenant-a", true),
                scoped_volume_test_route("/api/tenant-b", &tenant_b_dir, "/data/tenant-b", true),
                storage_broker_test_route(&shared_dir),
            ],
            ..IntegrityConfig::default_sealed()
        })
        .expect("zero-trust broker config should validate");

        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        let broker = Arc::clone(&state.storage_broker);
        let caller_route = config
            .sealed_route("/api/tenant-a")
            .expect("tenant-a route should remain sealed");
        let token = state
            .host_identity
            .sign_route(caller_route)
            .expect("caller token should sign");
        let app = build_app(state);

        let forged = app
            .clone()
            .oneshot(
                Request::post("/system/storage-broker?path=/data/tenant-a/forged.txt")
                    .header(TACHYON_IDENTITY_HEADER, "Bearer forged")
                    .body(Body::from("forged"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(forged.status(), StatusCode::FORBIDDEN);

        let accepted = app
            .clone()
            .oneshot(
                Request::post("/system/storage-broker?path=/data/tenant-a/state.txt")
                    .header(TACHYON_IDENTITY_HEADER, format!("Bearer {token}"))
                    .body(Body::from("allowed"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        assert!(
            broker.wait_for_volume_idle(&tenant_a_dir, Duration::from_secs(5)),
            "tenant-a broker queue should drain"
        );
        assert_eq!(
            fs::read_to_string(tenant_a_dir.join("state.txt"))
                .expect("authorized write should reach tenant-a volume"),
            "allowed"
        );

        let denied = app
            .oneshot(
                Request::post("/system/storage-broker?path=/data/tenant-b/state.txt")
                    .header(TACHYON_IDENTITY_HEADER, format!("Bearer {token}"))
                    .body(Body::from("blocked"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        let denied_body = denied
            .into_body()
            .collect()
            .await
            .expect("response body should collect")
            .to_bytes();
        assert!(
            String::from_utf8_lossy(&denied_body).contains("cannot broker writes"),
            "unexpected denial body: {:?}",
            denied_body
        );
        assert!(
            !tenant_b_dir.join("state.txt").exists(),
            "out-of-scope write should not create tenant-b data"
        );

        let _ = fs::remove_dir_all(shared_dir);
    }

    #[tokio::test]
    async fn volume_gc_tick_removes_stale_entries_from_short_lived_volume() {
        let volume_dir = unique_test_dir("tachyon-volume-gc");
        let stale_file = volume_dir.join("stale.txt");
        let stale_dir = volume_dir.join("stale-dir");
        fs::write(&stale_file, "stale").expect("stale file should be created");
        fs::create_dir_all(&stale_dir).expect("stale directory should be created");
        fs::write(stale_dir.join("nested.txt"), "stale").expect("nested file should be created");

        tokio::time::sleep(Duration::from_millis(1100)).await;

        let fresh_file = volume_dir.join("fresh.txt");
        fs::write(&fresh_file, "fresh").expect("fresh file should be created");

        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![ttl_managed_volume_route(&volume_dir, 1)];

        run_volume_gc_tick(Arc::new(build_test_runtime(config)))
            .await
            .expect("volume GC tick should complete");

        assert!(
            !stale_file.exists(),
            "stale file should be removed by the GC sweep"
        );
        assert!(
            !stale_dir.exists(),
            "stale directory should be removed by the GC sweep"
        );
        assert!(fresh_file.exists(), "fresh file should not be removed");

        let _ = fs::remove_dir_all(volume_dir);
    }

    #[tokio::test]
    async fn hibernating_ram_volume_swaps_out_and_restores_state() {
        let volume_dir = unique_test_dir("tachyon-ram-hibernate");
        let route = hibernating_ram_route(&volume_dir);
        let broker = Arc::new(StorageBrokerManager::default());
        let volume_manager = VolumeManager::default();

        {
            let _leases = volume_manager
                .acquire_route_volumes(&route, Arc::clone(&broker))
                .await
                .expect("initial route volume acquisition should succeed");
            fs::write(volume_dir.join("state.txt"), "hibernated state")
                .expect("state file should be written");
        }

        let managed = volume_manager
            .managed_volume_for_route(&route.path, "/app/data")
            .expect("managed volume should be registered");

        for _ in 0..50 {
            if managed.lifecycle() == ManagedVolumeLifecycle::OnDisk {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(managed.lifecycle(), ManagedVolumeLifecycle::OnDisk);
        assert!(
            broker
                .core_store
                .get(
                    store::CoreStoreBucket::HibernationState,
                    &managed_volume_id(&route.path, "/app/data"),
                )
                .expect("hibernation state lookup should succeed")
                .is_some(),
            "hibernation snapshot should be persisted in the core store"
        );
        assert!(
            !volume_dir.exists(),
            "active RAM volume directory should be released after hibernation"
        );

        let _restored = volume_manager
            .acquire_route_volumes(&route, Arc::clone(&broker))
            .await
            .expect("restoring hibernated volume should succeed");

        assert_eq!(managed.lifecycle(), ManagedVolumeLifecycle::Active);
        assert_eq!(
            fs::read_to_string(volume_dir.join("state.txt"))
                .expect("restored RAM volume should expose the original file"),
            "hibernated state"
        );

        let _ = fs::remove_dir_all(volume_dir);
    }

    #[test]
    fn validate_integrity_config_rejects_zero_max_concurrency() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![IntegrityRoute {
            path: "/api/guest-example".to_owned(),
            role: RouteRole::User,
            name: "guest-example".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            env: BTreeMap::new(),
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            resiliency: None,
            models: Vec::new(),
            domains: Vec::new(),
            min_instances: 0,
            max_concurrency: 0,
            volumes: Vec::new(),
        }];

        let error = validate_integrity_config(config)
            .expect_err("zero max_concurrency should fail validation");

        assert!(error
            .to_string()
            .contains("must set `max_concurrency` above zero"));
    }

    #[test]
    fn validate_integrity_config_accepts_route_resiliency_policy() {
        let mut config = IntegrityConfig::default_sealed();
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.resiliency = Some(ResiliencyConfig {
            timeout_ms: Some(500),
            retry_policy: Some(RetryPolicy {
                max_retries: 5,
                retry_on: vec![503, 502, 503],
            }),
        });
        config.routes = vec![route];

        let config =
            validate_integrity_config(config).expect("resiliency-enabled route should validate");
        let route = config
            .sealed_route("/api/guest-example")
            .expect("route should remain sealed");

        assert_eq!(
            route.resiliency,
            Some(ResiliencyConfig {
                timeout_ms: Some(500),
                retry_policy: Some(RetryPolicy {
                    max_retries: 5,
                    retry_on: vec![502, 503],
                }),
            })
        );
    }

    #[test]
    fn validate_integrity_config_rejects_retry_policy_without_statuses() {
        let mut config = IntegrityConfig::default_sealed();
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.resiliency = Some(ResiliencyConfig {
            timeout_ms: None,
            retry_policy: Some(RetryPolicy {
                max_retries: 2,
                retry_on: Vec::new(),
            }),
        });
        config.routes = vec![route];

        let error = validate_integrity_config(config)
            .expect_err("retry policy without retry_on statuses should fail validation");

        assert!(error
            .to_string()
            .contains("must configure at least one `resiliency.retry_policy.retry_on` status"));
    }

    #[cfg(not(feature = "resiliency"))]
    #[tokio::test]
    async fn route_resiliency_config_is_overhead_free_when_feature_is_disabled() {
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![resiliency_test_route(Some(ResiliencyConfig {
                timeout_ms: Some(500),
                retry_policy: Some(RetryPolicy {
                    max_retries: 5,
                    retry_on: vec![503],
                }),
            }))],
            ..IntegrityConfig::default_sealed()
        })
        .expect("resiliency route should validate without the feature");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

        let response = app
            .oneshot(
                Request::post("/api/guest-flaky")
                    .body(Body::from("force-fail"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[cfg(feature = "resiliency")]
    #[tokio::test]
    async fn route_resiliency_timeout_applies_to_guest_execution() {
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![resiliency_test_route(Some(ResiliencyConfig {
                timeout_ms: Some(50),
                retry_policy: None,
            }))],
            ..IntegrityConfig::default_sealed()
        })
        .expect("resiliency route should validate");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

        let response = app
            .oneshot(
                Request::post("/api/guest-flaky")
                    .body(Body::from("sleep:2000"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();

        assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
        assert!(String::from_utf8_lossy(&body).contains("timed out after 50ms"));
    }

    #[test]
    fn embedded_integrity_payload_is_a_valid_runtime_config() {
        let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
            .expect("embedded payload should deserialize into an integrity config");
        let config = validate_integrity_config(config).expect("embedded config should validate");

        assert_eq!(config.guest_fuel_budget, DEFAULT_GUEST_FUEL_BUDGET);
        assert_eq!(
            config
                .sealed_route("/metrics")
                .expect("embedded config should seal the system metrics route")
                .role,
            RouteRole::System
        );
        assert_eq!(
            config
                .sealed_route("/api/guest-example")
                .expect("embedded config should seal the example route")
                .allowed_secrets,
            vec!["DB_PASS".to_owned()]
        );
        assert_eq!(
            config
                .sealed_route("/api/guest-example")
                .expect("embedded config should seal the example route")
                .min_instances,
            0
        );
        assert_eq!(
            config
                .sealed_route("/api/guest-example")
                .expect("embedded config should seal the example route")
                .max_concurrency,
            DEFAULT_ROUTE_MAX_CONCURRENCY
        );
        assert!(config.sealed_route("/api/guest-example").is_some());
        assert!(config.sealed_route("/api/guest-loop").is_some());
        assert!(config.sealed_route("/api/guest-csharp").is_some());
        assert!(config.sealed_route("/api/guest-java").is_some());
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
    fn guest_ai_is_gated_behind_ai_inference_feature() {
        assert!(requires_ai_inference_feature("guest-ai"));
        assert!(!requires_ai_inference_feature("guest-example"));
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
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum PeerPressureState {
    #[default]
    Idle,
    Caution,
    Saturated,
}
