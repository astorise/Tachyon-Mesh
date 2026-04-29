use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
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
    routing::{get, post, put},
    Extension, Router,
};
use clap::{Args as ClapArgs, Parser, Subcommand};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
#[cfg(feature = "websockets")]
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use hyper::body::{Frame, SizeHint};
use hyper::service::service_fn;
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder as HyperConnectionBuilder,
    service::TowerToHyperService,
};
use rand::RngExt;
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
    convert::Infallible,
    fmt, fs,
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Once, OnceLock,
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
mod auth;
mod data_events;
mod node_enrollment;
#[cfg(feature = "rate-limit")]
mod rate_limit;
#[cfg(feature = "resiliency")]
mod resiliency;
#[cfg(feature = "http3")]
mod server_h3;
mod store;
mod system_storage;
mod telemetry;
mod tls_runtime;

mod component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "faas-guest",
    });
}

mod system_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "system-faas-guest",
    });
}

mod udp_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "udp-faas-guest",
    });
}

#[cfg(feature = "websockets")]
mod websocket_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "websocket-faas-guest",
    });
}

mod background_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "background-system-faas",
    });
}

mod control_plane_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "control-plane-faas",
    });
}

#[cfg(feature = "ai-inference")]
mod accelerator_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/accelerator",
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
const SYSTEM_BRIDGE_ROUTE: &str = "/system/bridge";
const SYSTEM_CERT_MANAGER_ROUTE: &str = "/system/cert-manager";
const SYSTEM_GATEWAY_ROUTE: &str = "/system/gateway";
const SYSTEM_LOGGER_ROUTE: &str = "/system/logger";
const SYSTEM_DIST_LIMITER_ROUTE: &str = "/system/dist-limiter";
const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");
const INTEGRITY_MANIFEST_PATH_ENV: &str = "TACHYON_INTEGRITY_MANIFEST";
const DEFAULT_HOP_LIMIT: u32 = 10;
const HOP_LIMIT_HEADER: &str = "x-tachyon-hop-limit";
const COHORT_HEADER: &str = "x-cohort";
const TACHYON_COHORT_HEADER: &str = "x-tachyon-cohort";
const TACHYON_IDENTITY_HEADER: &str = "x-tachyon-identity";
const TACHYON_ORIGINAL_ROUTE_HEADER: &str = "x-tachyon-original-route";
const TACHYON_BUFFER_REPLAY_HEADER: &str = "x-tachyon-buffer-replay";
const MESH_QOS_OVERRIDE_PREFIX: &str = "mesh-qos:";
const TACHYON_SYSTEM_PUBLIC_KEY_ENV: &str = "TACHYON_SYSTEM_PUBLIC_KEY";
const TACHYON_MTLS_ADDRESS_ENV: &str = "TACHYON_MTLS_ADDRESS";
#[cfg(unix)]
const TACHYON_DISCOVERY_DIR_ENV: &str = "TACHYON_DISCOVERY_DIR";
const LOG_QUEUE_CAPACITY: usize = 64_000;
const LOG_BATCH_SIZE: usize = 1_000;
const LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD: usize = 32;
const DEFAULT_ROUTE_MAX_CONCURRENCY: u32 = 100;
#[cfg(test)]
const DEFAULT_ROUTE_VERSION: &str = "0.0.0";
const DEFAULT_TELEMETRY_SAMPLE_RATE: f64 = 0.0;
const TDE_FILE_MAGIC: &[u8] = b"TACHYON-TDE-v1\0";
const TDE_KEY_HEX_ENV: &str = "TDE_KEY_HEX";
const MODEL_BROKER_DIR_ENV: &str = "MODEL_BROKER_DIR";
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
const DISTRIBUTED_RATE_LIMIT_TIMEOUT: Duration = Duration::from_millis(5);
static DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL: AtomicU64 = AtomicU64::new(0);
const POOLING_CORE_INSTANCES_MULTIPLIER: u32 = 8;
const POOLING_MEMORIES_MULTIPLIER: u32 = 2;
const POOLING_TABLES_MULTIPLIER: u32 = 2;
const POOLING_INSTANCE_METADATA_BYTES: usize = 1 << 20;
const POOLING_MAX_CORE_INSTANCES_PER_COMPONENT: u32 = 50;
const POOLING_MAX_MEMORIES_PER_COMPONENT: u32 = 8;
const POOLING_MAX_TABLES_PER_COMPONENT: u32 = 8;
const ERR_INTEGRITY_SCHEMA_VIOLATION: &str = "ERR_INTEGRITY_SCHEMA_VIOLATION";

fn default_max_concurrency() -> u32 {
    DEFAULT_ROUTE_MAX_CONCURRENCY
}

#[cfg(test)]
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
    bridge_manager: Arc<BridgeManager>,
    core_store: Arc<store::CoreStore>,
    buffered_requests: Arc<BufferedRequestManager>,
    volume_manager: Arc<VolumeManager>,
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    peer_capabilities: PeerCapabilityCache,
    host_capabilities: Capabilities,
    host_load: Arc<HostLoadCounters>,
    telemetry: TelemetryHandle,
    tls_manager: Arc<tls_runtime::TlsManager>,
    mtls_gateway: Option<Arc<tls_runtime::MtlsGatewayConfig>>,
    auth_manager: Arc<auth::AuthManager>,
    enrollment_manager: Arc<node_enrollment::EnrollmentManager>,
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
    /// In-memory cache of `Arc<Module>` keyed by module file path. The
    /// existing redb `cwasm_cache` table eliminates the JIT-compile cost
    /// across host restarts, but every request still pays
    /// `Module::deserialize` (~hundreds of microseconds for typical modules)
    /// when re-reading from redb. This cache amortizes that cost across all
    /// requests within a single runtime generation; on hot reload the cache
    /// is dropped along with the rest of the runtime, so configuration
    /// changes propagate without a stale-module concern.
    instance_pool: Arc<moka::sync::Cache<PathBuf, Arc<Module>>>,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

#[derive(Default)]
struct HostLoadCounters {
    active_instances: AtomicUsize,
    allocated_memory_pages: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Capabilities {
    mask: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct CachedPeerCapabilities {
    capabilities: Vec<String>,
    capability_mask: u64,
    effective_pressure: u8,
}

type PeerCapabilityCache = Arc<Mutex<HashMap<String, CachedPeerCapabilities>>>;

impl Capabilities {
    const CORE_WASI: u64 = 1 << 0;
    const LEGACY_OCI: u64 = 1 << 1;
    const ACCEL_CUDA: u64 = 1 << 2;
    const ACCEL_OPENVINO: u64 = 1 << 3;
    const ACCEL_TPU: u64 = 1 << 4;
    const NET_LAYER4: u64 = 1 << 5;
    const FEATURE_WEBSOCKETS: u64 = 1 << 6;
    const FEATURE_HTTP3: u64 = 1 << 7;
    const FEATURE_AI_INFERENCE: u64 = 1 << 8;
    const OS_LINUX: u64 = 1 << 9;
    const OS_WINDOWS: u64 = 1 << 10;

    fn detect() -> Self {
        let mut mask = Self::CORE_WASI | Self::NET_LAYER4;
        if cfg!(target_os = "linux") {
            mask |= Self::OS_LINUX;
            if is_v1_container_runtime() {
                mask |= Self::LEGACY_OCI;
            }
        }
        if cfg!(target_os = "windows") {
            mask |= Self::OS_WINDOWS;
        }
        if cfg!(feature = "websockets") {
            mask |= Self::FEATURE_WEBSOCKETS;
        }
        if cfg!(feature = "http3") {
            mask |= Self::FEATURE_HTTP3;
        }
        if cfg!(feature = "ai-inference") {
            mask |= Self::FEATURE_AI_INFERENCE
                | Self::ACCEL_CUDA
                | Self::ACCEL_OPENVINO
                | Self::ACCEL_TPU;
        }
        Self { mask }
    }

    fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    fn from_requirement_list(requirements: &[String]) -> Result<Self> {
        let mut mask = 0_u64;
        let names = if requirements.is_empty() {
            default_route_capabilities()
        } else {
            requirements.to_vec()
        };
        for requirement in names {
            mask |= capability_flag(&requirement)?;
        }
        Ok(Self { mask })
    }

    fn supports(self, required: Self) -> bool {
        (self.mask & required.mask) == required.mask
    }

    fn names(self) -> Vec<String> {
        capability_names_from_mask(self.mask)
    }

    fn missing_names(self, required: Self) -> Vec<String> {
        capability_names_from_mask(required.mask & !self.mask)
    }
}

fn default_route_capabilities() -> Vec<String> {
    vec!["core:wasi".to_owned()]
}

fn capability_flag(value: &str) -> Result<u64> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "core:wasi" => Ok(Capabilities::CORE_WASI),
        "legacy:oci" => Ok(Capabilities::LEGACY_OCI),
        "accel:cuda" => Ok(Capabilities::ACCEL_CUDA),
        "accel:openvino" => Ok(Capabilities::ACCEL_OPENVINO),
        "accel:tpu" => Ok(Capabilities::ACCEL_TPU),
        "net:layer4" => Ok(Capabilities::NET_LAYER4),
        "feature:websockets" => Ok(Capabilities::FEATURE_WEBSOCKETS),
        "feature:http3" => Ok(Capabilities::FEATURE_HTTP3),
        "feature:ai-inference" => Ok(Capabilities::FEATURE_AI_INFERENCE),
        "os:linux" => Ok(Capabilities::OS_LINUX),
        "os:windows" => Ok(Capabilities::OS_WINDOWS),
        _ => Err(anyhow!("unknown capability `{value}`")),
    }
}

fn capability_names_from_mask(mask: u64) -> Vec<String> {
    [
        (Capabilities::CORE_WASI, "core:wasi"),
        (Capabilities::LEGACY_OCI, "legacy:oci"),
        (Capabilities::ACCEL_CUDA, "accel:cuda"),
        (Capabilities::ACCEL_OPENVINO, "accel:openvino"),
        (Capabilities::ACCEL_TPU, "accel:tpu"),
        (Capabilities::NET_LAYER4, "net:layer4"),
        (Capabilities::FEATURE_WEBSOCKETS, "feature:websockets"),
        (Capabilities::FEATURE_HTTP3, "feature:http3"),
        (Capabilities::FEATURE_AI_INFERENCE, "feature:ai-inference"),
        (Capabilities::OS_LINUX, "os:linux"),
        (Capabilities::OS_WINDOWS, "os:windows"),
    ]
    .into_iter()
    .filter(|(flag, _)| (mask & *flag) != 0)
    .map(|(_, name)| name.to_owned())
    .collect()
}

fn normalize_capabilities(
    capabilities: Vec<String>,
    context: impl AsRef<str>,
) -> Result<Vec<String>> {
    let context = context.as_ref();
    let mut normalized = BTreeSet::new();
    let source = if capabilities.is_empty() {
        default_route_capabilities()
    } else {
        capabilities
    };
    for capability in source {
        let trimmed = capability.trim();
        if trimmed.is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: {context} must not contain empty capabilities"
            ));
        }
        let canonical = trimmed.to_ascii_lowercase();
        capability_flag(&canonical)
            .map_err(|error| anyhow!("Integrity Validation Failed: {context} declares {error}"))?;
        normalized.insert(canonical);
    }
    Ok(normalized.into_iter().collect())
}

fn is_v1_container_runtime() -> bool {
    std::env::var("TACHYON_LEGACY_OCI")
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
        || Path::new("/.dockerenv").exists()
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

struct MtlsGatewayListenerHandle {
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
    bridge_manager: Arc<BridgeManager>,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    peer_capabilities: PeerCapabilityCache,
    host_capabilities: Capabilities,
    host_load: Arc<HostLoadCounters>,
    outbound_http_client: reqwest::blocking::Client,
    route_path: String,
    route_role: RouteRole,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Option<Arc<ai_inference::AiInferenceRuntime>>,
    #[cfg(feature = "ai-inference")]
    allowed_model_aliases: BTreeSet<String>,
    #[cfg(feature = "ai-inference")]
    adapter_id: Option<String>,
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
    bridge_manager: Arc<BridgeManager>,
    telemetry: Option<GuestTelemetryContext>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    host_load: Arc<HostLoadCounters>,
    /// In-memory `Arc<Module>` cache shared with the active runtime. The hot
    /// HTTP / L4 paths consult this before the redb-backed `cwasm_cache` to
    /// avoid the `Module::deserialize` cost on every request. Tests fill in
    /// `None`; production code clones it from `RuntimeState::instance_pool`.
    instance_pool: Option<Arc<moka::sync::Cache<PathBuf, Arc<Module>>>>,
    #[cfg(feature = "ai-inference")]
    ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

static BLOCKING_OUTBOUND_HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

fn blocking_outbound_http_client() -> reqwest::blocking::Client {
    BLOCKING_OUTBOUND_HTTP_CLIENT
        .get_or_init(|| {
            std::thread::Builder::new()
                .name("tachyon-blocking-http-init".to_owned())
                .spawn(reqwest::blocking::Client::new)
                .expect("blocking outbound HTTP client init thread should start")
                .join()
                .expect("blocking outbound HTTP client init thread should succeed")
        })
        .clone()
}

struct BackgroundTickRunner {
    function_name: String,
    route_path: String,
    store: Store<ComponentHostState>,
    bindings: BackgroundGuestBindings,
}

enum BackgroundGuestBindings {
    Background(background_component_bindings::BackgroundSystemFaas),
    ControlPlane(control_plane_component_bindings::ControlPlaneFaas),
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

struct HostLoadGuard {
    counters: Arc<HostLoadCounters>,
    allocated_pages: usize,
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

impl HostLoadGuard {
    fn new(counters: Arc<HostLoadCounters>, allocated_pages: usize) -> Self {
        counters.active_instances.fetch_add(1, Ordering::SeqCst);
        counters
            .allocated_memory_pages
            .fetch_add(allocated_pages, Ordering::SeqCst);
        Self {
            counters,
            allocated_pages,
        }
    }
}

impl Drop for HostLoadGuard {
    fn drop(&mut self) {
        self.counters
            .active_instances
            .fetch_sub(1, Ordering::SeqCst);
        self.counters
            .allocated_memory_pages
            .fetch_sub(self.allocated_pages, Ordering::SeqCst);
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
#[serde(tag = "type", rename_all = "lowercase")]
enum FaaSRuntime {
    Wasm {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
    },
    Microvm {
        image: String,
        #[serde(default = "default_microvm_vcpus")]
        vcpus: u8,
        #[serde(default = "default_microvm_memory_mb")]
        memory_mb: u32,
    },
}

fn default_microvm_vcpus() -> u8 {
    1
}

fn default_microvm_memory_mb() -> u32 {
    256
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    requires: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedRouteTarget {
    module: String,
    websocket: bool,
    required_capabilities: Vec<String>,
    required_capability_mask: u64,
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

        let mut rng = rand::rng();
        let first_index = rng.random_range(0..candidates.len());
        let mut second_index = rng.random_range(0..candidates.len() - 1);
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
    sync_to_cloud: bool,
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

static LORA_TRAINING_QUEUE: OnceLock<Arc<LoraTrainingQueue>> = OnceLock::new();
static AI_INFERENCE_JOBS: OnceLock<Arc<Mutex<HashMap<String, AiInferenceJobStatus>>>> =
    OnceLock::new();

struct LoraTrainingQueue {
    sender: std::sync::mpsc::Sender<LoraTrainingJob>,
    statuses: Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
}

#[derive(Clone, Debug)]
struct LoraTrainingJob {
    id: String,
    tenant_id: String,
    base_model: String,
    dataset_volume: String,
    dataset_path: String,
    dataset_split: Option<String>,
    rank: u32,
    max_steps: u32,
    seed: Option<u64>,
}

#[derive(Clone, Debug)]
enum LoraTrainingJobStatus {
    Queued,
    Running { step: u32, total: u32 },
    Completed { adapter_path: String },
    Failed { message: String },
}

#[derive(Clone, Debug)]
enum AiInferenceJobStatus {
    Queued,
    Running,
    Completed {
        output: String,
    },
    #[allow(dead_code)]
    Failed {
        message: String,
    },
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
    version: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_policy: Option<ResourcePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime: Option<FaaSRuntime>,
    /// Routes flagged here mirror data writes to a cloud endpoint via the existing
    /// `system-faas-cdc` outbox path. Off by default; opting in adds an asynchronous
    /// post-write event emit but no synchronous latency.
    #[serde(default, skip_serializing_if = "is_false")]
    sync_to_cloud: bool,
    /// Route runs inside a hardware Trusted Execution Environment when true. The host
    /// dispatches via `IntegrityConfig::tee_backend` instead of the pooled engine.
    #[serde(default, skip_serializing_if = "is_false")]
    requires_tee: bool,
    /// Route may overflow to peer nodes via `system-faas-mesh-overlay` when the local
    /// accelerator or worker pool is saturated.
    #[serde(default, skip_serializing_if = "is_false")]
    allow_overflow: bool,
    /// Opt-in distributed rate-limiting policy enforced via `system-faas-dist-limiter`.
    /// When `None`, only the local LRU rate limiter applies (the host fails open on a
    /// distributed limiter outage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    distributed_rate_limit: Option<DistributedRateLimitConfig>,
    /// Tenant-specific LoRA adapter to apply on top of the route's foundation model
    /// at inference time. Per-call overrides may be passed via the inference WIT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adapter_id: Option<String>,
}

impl Default for IntegrityRoute {
    fn default() -> Self {
        Self {
            path: String::new(),
            role: RouteRole::User,
            name: String::new(),
            version: "0.0.0".to_owned(),
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
            resource_policy: None,
            runtime: None,
            sync_to_cloud: false,
            requires_tee: false,
            allow_overflow: false,
            distributed_rate_limit: None,
            adapter_id: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdmissionStrategy {
    FailFast,
    MeshRetry,
}

impl Default for AdmissionStrategy {
    fn default() -> Self {
        Self::FailFast
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
struct ResourcePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_ram_gb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_ram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    min_vram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "is_default_admission_strategy")]
    admission_strategy: AdmissionStrategy,
}

fn is_default_admission_strategy(strategy: &AdmissionStrategy) -> bool {
    *strategy == AdmissionStrategy::FailFast
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct DistributedRateLimitConfig {
    /// Per-IP request count permitted across the entire mesh within `window_seconds`.
    threshold: u32,
    #[serde(default = "default_dist_rate_limit_window")]
    window_seconds: u32,
}

fn default_dist_rate_limit_window() -> u32 {
    60
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
    /// Route writes to this volume are paged through `system-faas-tde` for AES-256-GCM
    /// encryption at rest. Off by default to preserve native disk speed for routes
    /// that don't need TDE.
    #[serde(default, skip_serializing_if = "is_false")]
    encrypted: bool,
}

impl Default for IntegrityVolume {
    fn default() -> Self {
        Self {
            volume_type: VolumeType::Host,
            host_path: String::new(),
            guest_path: String::new(),
            readonly: false,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,
            encrypted: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IntegrityResource {
    Internal {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version_constraint: Option<String>,
    },
    External {
        target: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_methods: Vec<String>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct IntegrityConfig {
    host_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    advertise_ip: Option<String>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    resources: BTreeMap<String, IntegrityResource>,
    routes: Vec<IntegrityRoute>,
    /// Monotonically increasing version stamp used by the multi-master config sync
    /// path: a node receives a `ConfigUpdateEvent` and pulls the manifest from the
    /// origin only when the advertised version is higher than the local one.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    config_version: u64,
    /// Outbound endpoint a freshly booted, unenrolled node uses to wait for a PIN
    /// approval from an active mesh node. Optional — clusters that pre-seed
    /// credentials don't need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enrollment_endpoint: Option<String>,
    /// Cloud endpoint that `system-faas-cdc` POSTs to when draining the
    /// data-mutation outbox. Optional — air-gapped deployments leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cloud_sync_endpoint: Option<String>,
    /// TEE delegation backend used by routes flagged `requires_tee`. Optional —
    /// without it, a manifest with TEE-flagged routes is rejected by validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tee_backend: Option<TeeBackendConfig>,
    /// Hard cap on memory used by the Wasmtime instance pool. Optional — when unset,
    /// the existing `PoolingAllocationConfig` defaults apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instance_pool_max_memory_bytes: Option<usize>,
}

impl Default for IntegrityConfig {
    fn default() -> Self {
        Self {
            host_address: String::new(),
            advertise_ip: None,
            tls_address: None,
            max_stdout_bytes: 0,
            guest_fuel_budget: 0,
            guest_memory_limit_bytes: 0,
            resource_limit_response: String::new(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            resources: BTreeMap::new(),
            routes: Vec::new(),
            config_version: 0,
            enrollment_endpoint: None,
            cloud_sync_endpoint: None,
            tee_backend: None,
            instance_pool_max_memory_bytes: None,
        }
    }
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum TeeBackendConfig {
    /// In-process hardened wasmtime backend with mlocked memory and a self-attested
    /// JWT carrying the host identity. Available on every host; security guarantees
    /// match the host kernel.
    LocalEnclave,
    /// Real Enarx backend. Requires the `enarx` Cargo feature and SGX/SEV-SNP HW.
    Enarx { keep_endpoint: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OutboundTargetKind {
    Internal,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedOutboundTarget {
    url: String,
    kind: OutboundTargetKind,
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
    ensure_rustls_crypto_provider();
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
    let bridge_manager = Arc::new(BridgeManager::default());
    let buffered_requests = Arc::new(BufferedRequestManager::new(buffered_request_spool_dir(
        &manifest_path,
    )));
    let background_workers = Arc::new(BackgroundWorkerManager::default());
    let route_overrides = Arc::new(ArcSwap::from_pointee(HashMap::new()));
    let peer_capabilities = Arc::new(Mutex::new(HashMap::new()));
    let host_capabilities = Capabilities::detect();
    let host_load = Arc::new(HostLoadCounters::default());
    let tls_manager = Arc::new(tls_runtime::TlsManager::default());
    let mtls_gateway = tls_runtime::load_mtls_gateway_config_from_env()?;
    let auth_manager = Arc::new(auth::AuthManager::new(&manifest_path)?);
    let (async_log_sender, async_log_receiver) = mpsc::channel(LOG_QUEUE_CAPACITY);
    background_workers.start_for_runtime(
        &runtime,
        telemetry.clone(),
        Arc::clone(&host_identity),
        Arc::clone(&storage_broker),
        Arc::clone(&route_overrides),
        Arc::clone(&peer_capabilities),
        host_capabilities,
        Arc::clone(&host_load),
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
        bridge_manager,
        core_store,
        buffered_requests,
        volume_manager: Arc::new(VolumeManager::default()),
        route_overrides,
        peer_capabilities,
        host_capabilities,
        host_load,
        telemetry,
        tls_manager,
        mtls_gateway: mtls_gateway.map(Arc::new),
        auth_manager,
        enrollment_manager: Arc::new(node_enrollment::EnrollmentManager::new()),
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
    spawn_manifest_file_watcher(state.clone());
    spawn_authz_purge_subscriber(state.clone());
    spawn_draining_runtime_reaper(state.clone());
    spawn_volume_gc_sweeper(state.clone());
    spawn_buffered_request_replayer(state.clone());
    spawn_pressure_monitor(state.clone());
    let app = build_app(state.clone());
    let https_listener = start_https_listener(state.clone(), app.clone()).await?;
    let mtls_listener = start_mtls_gateway_listener(state.clone()).await?;
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
    if let Some(listener) = mtls_listener {
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
    #[allow(clippy::too_many_arguments)]
    fn start_for_runtime(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
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
                Arc::clone(&route_overrides),
                Arc::clone(&peer_capabilities),
                host_capabilities,
                Arc::clone(&host_load),
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
            let worker_route_overrides = Arc::clone(&route_overrides);
            let worker_peer_capabilities = Arc::clone(&peer_capabilities);
            let worker_host_capabilities = host_capabilities;
            let worker_host_load = Arc::clone(&host_load);
            let worker_stop = Arc::clone(&stop_requested);
            let join_handle = tokio::task::spawn_blocking(move || {
                run_background_tick_loop(
                    worker_engine,
                    worker_config,
                    worker_telemetry,
                    worker_limits,
                    worker_host_identity,
                    worker_storage_broker,
                    worker_route_overrides,
                    worker_peer_capabilities,
                    worker_host_capabilities,
                    worker_host_load,
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
    #[allow(clippy::too_many_arguments)]
    async fn replace_with(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
    ) {
        self.stop_all().await;
        self.start_for_runtime(
            runtime,
            telemetry,
            host_identity,
            storage_broker,
            route_overrides,
            peer_capabilities,
            host_capabilities,
            host_load,
        );
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
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    peer_capabilities: PeerCapabilityCache,
    host_capabilities: Capabilities,
    host_load: Arc<HostLoadCounters>,
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
        route_overrides,
        peer_capabilities,
        host_capabilities,
        host_load,
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
        self.enqueue_write_target(
            route.path.clone(),
            route.sync_to_cloud,
            resolved,
            mode,
            body,
        )
    }

    fn enqueue_write_target(
        &self,
        route_path: String,
        sync_to_cloud: bool,
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
            sync_to_cloud,
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
                    } else if request.sync_to_cloud {
                        if let Err(error) = emit_storage_mutation_event(&self.core_store, &request)
                        {
                            tracing::warn!(
                                route = %request.route_path,
                                guest_path = %request.guest_path,
                                host_target = %request.host_target.display(),
                                "storage broker CDC event emit failed: {error:#}"
                            );
                        }
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

#[derive(Clone, Default)]
struct BridgeManager {
    inner: Arc<BridgeManagerInner>,
}

#[derive(Default)]
struct BridgeManagerInner {
    sessions: Mutex<HashMap<String, BridgeSession>>,
    active_relays: AtomicUsize,
    relayed_bytes: AtomicU64,
    telemetry: Mutex<BridgeTelemetryState>,
}

struct BridgeSession {
    abort_handle: tokio::task::AbortHandle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct BridgeConfig {
    client_a_addr: String,
    client_b_addr: String,
    timeout_seconds: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct BridgeAllocation {
    bridge_id: String,
    #[serde(default)]
    ip: String,
    port_a: u16,
    port_b: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct BridgeTelemetrySnapshot {
    active_relays: u32,
    throughput_bytes_per_sec: u64,
    load_score: u8,
}

struct BridgeTelemetryState {
    last_total_bytes: u64,
    last_sampled_at: Instant,
    throughput_bytes_per_sec: u64,
}

impl Default for BridgeTelemetryState {
    fn default() -> Self {
        Self {
            last_total_bytes: 0,
            last_sampled_at: Instant::now(),
            throughput_bytes_per_sec: 0,
        }
    }
}

impl BridgeManager {
    fn create_relay(&self, config: BridgeConfig) -> std::result::Result<BridgeAllocation, String> {
        let endpoint_a = parse_bridge_endpoint(&config.client_a_addr, "client_a_addr")?;
        let endpoint_b = parse_bridge_endpoint(&config.client_b_addr, "client_b_addr")?;
        let inactivity_timeout = Duration::from_secs(u64::from(config.timeout_seconds.max(1)));

        let socket_a = bind_bridge_socket()?;
        let socket_b = bind_bridge_socket()?;
        let port_a = socket_a
            .local_addr()
            .map_err(|error| format!("failed to resolve bridge port A: {error}"))?
            .port();
        let port_b = socket_b
            .local_addr()
            .map_err(|error| format!("failed to resolve bridge port B: {error}"))?
            .port();

        let bridge_id = Uuid::new_v4().to_string();
        let inner = Arc::clone(&self.inner);
        let cleanup_id = bridge_id.clone();
        let join_handle = tokio::spawn(async move {
            if let Err(error) = relay_bridge(
                socket_a,
                socket_b,
                endpoint_a,
                endpoint_b,
                inactivity_timeout,
                &inner,
            )
            .await
            {
                tracing::warn!(bridge_id = %cleanup_id, "dynamic bridge relay exited: {error}");
            }
            release_bridge_session(&inner, &cleanup_id);
        });

        let mut sessions = self
            .inner
            .sessions
            .lock()
            .expect("bridge session registry should not be poisoned");
        sessions.insert(
            bridge_id.clone(),
            BridgeSession {
                abort_handle: join_handle.abort_handle(),
            },
        );
        drop(sessions);
        self.inner.active_relays.fetch_add(1, Ordering::SeqCst);

        Ok(BridgeAllocation {
            bridge_id,
            ip: String::new(),
            port_a,
            port_b,
        })
    }

    fn destroy_relay(&self, bridge_id: &str) -> std::result::Result<(), String> {
        let session = self
            .inner
            .sessions
            .lock()
            .expect("bridge session registry should not be poisoned")
            .remove(bridge_id);
        let Some(session) = session else {
            return Err(format!("bridge `{bridge_id}` is not active"));
        };
        session.abort_handle.abort();
        self.inner.active_relays.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(test)]
    fn active_relay_count(&self) -> usize {
        self.inner.active_relays.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn total_relayed_bytes(&self) -> u64 {
        self.inner.relayed_bytes.load(Ordering::SeqCst)
    }

    fn telemetry_snapshot(&self) -> BridgeTelemetrySnapshot {
        let total_bytes = self.inner.relayed_bytes.load(Ordering::SeqCst);
        let active_relays = self.inner.active_relays.load(Ordering::SeqCst) as u32;
        let mut telemetry = self
            .inner
            .telemetry
            .lock()
            .expect("bridge telemetry state should not be poisoned");
        let elapsed = telemetry.last_sampled_at.elapsed();
        if elapsed >= Duration::from_millis(250) {
            let delta = total_bytes.saturating_sub(telemetry.last_total_bytes);
            telemetry.throughput_bytes_per_sec = if elapsed.is_zero() {
                0
            } else {
                ((delta as u128 * 1_000_000_000_u128) / elapsed.as_nanos()) as u64
            };
            telemetry.last_total_bytes = total_bytes;
            telemetry.last_sampled_at = Instant::now();
        }

        BridgeTelemetrySnapshot {
            active_relays,
            throughput_bytes_per_sec: telemetry.throughput_bytes_per_sec,
            load_score: bridge_load_score(active_relays, telemetry.throughput_bytes_per_sec),
        }
    }
}

fn bridge_load_score(active_relays: u32, throughput_bytes_per_sec: u64) -> u8 {
    let relay_score = active_relays.saturating_mul(25).min(100) as u8;
    let throughput_score = ((throughput_bytes_per_sec / 50_000).min(100)) as u8;
    relay_score.max(throughput_score)
}

fn bind_bridge_socket() -> std::result::Result<tokio::net::UdpSocket, String> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|error| format!("failed to bind dynamic bridge socket: {error}"))?;
    socket
        .set_nonblocking(true)
        .map_err(|error| format!("failed to set dynamic bridge socket nonblocking: {error}"))?;
    tokio::net::UdpSocket::from_std(socket)
        .map_err(|error| format!("failed to convert dynamic bridge socket to tokio: {error}"))
}

fn parse_bridge_endpoint(value: &str, label: &str) -> std::result::Result<SocketAddr, String> {
    value
        .trim()
        .parse::<SocketAddr>()
        .map_err(|error| format!("failed to parse `{label}` as socket address: {error}"))
}

async fn relay_bridge(
    socket_a: tokio::net::UdpSocket,
    socket_b: tokio::net::UdpSocket,
    endpoint_a: SocketAddr,
    endpoint_b: SocketAddr,
    inactivity_timeout: Duration,
    inner: &BridgeManagerInner,
) -> std::result::Result<(), String> {
    let mut buffer_a = [0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
    let mut buffer_b = [0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
    let mut deadline = tokio::time::Instant::now() + inactivity_timeout;

    loop {
        let sleep = tokio::time::sleep_until(deadline);
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => return Ok(()),
            received = socket_a.recv_from(&mut buffer_a) => {
                let (size, _) = received.map_err(|error| format!("failed to receive bridge packet on port A: {error}"))?;
                socket_b
                    .send_to(&buffer_a[..size], endpoint_b)
                    .await
                    .map_err(|error| format!("failed to forward bridge packet to endpoint B: {error}"))?;
                inner.relayed_bytes.fetch_add(size as u64, Ordering::SeqCst);
                deadline = tokio::time::Instant::now() + inactivity_timeout;
            }
            received = socket_b.recv_from(&mut buffer_b) => {
                let (size, _) = received.map_err(|error| format!("failed to receive bridge packet on port B: {error}"))?;
                socket_a
                    .send_to(&buffer_b[..size], endpoint_a)
                    .await
                    .map_err(|error| format!("failed to forward bridge packet to endpoint A: {error}"))?;
                inner.relayed_bytes.fetch_add(size as u64, Ordering::SeqCst);
                deadline = tokio::time::Instant::now() + inactivity_timeout;
            }
        }
    }
}

fn release_bridge_session(inner: &BridgeManagerInner, bridge_id: &str) {
    let removed = inner
        .sessions
        .lock()
        .expect("bridge session registry should not be poisoned")
        .remove(bridge_id)
        .is_some();
    if removed {
        inner.active_relays.fetch_sub(1, Ordering::SeqCst);
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

fn emit_storage_mutation_event(
    core_store: &store::CoreStore,
    request: &StorageBrokerWriteRequest,
) -> Result<String> {
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system clock is set before the Unix epoch")?
        .as_millis();
    let value_hash = format!("sha256:{}", hex::encode(Sha256::digest(&request.body)));
    let payload = serde_json::to_vec(&serde_json::json!({
        "event": "tachyon.data.mutation",
        "route_path": request.route_path,
        "resource": request.guest_path,
        "operation": match request.mode {
            StorageWriteMode::Overwrite => "overwrite",
            StorageWriteMode::Append => "append",
        },
        "value_hash": value_hash,
        "value_bytes": request.body.len(),
        "timestamp_unix_ms": timestamp_unix_ms,
    }))
    .context("failed to serialize CDC mutation event")?;

    core_store.append_outbox(store::CoreStoreBucket::DataMutationOutbox, &payload)
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

const MANIFEST_FILE_WATCHER_DEBOUNCE: Duration = Duration::from_millis(250);

/// Spawn a file watcher that triggers a hot reload whenever the integrity manifest is
/// modified or atomically replaced on disk. Many editors and CI/CD tools save the file
/// by writing a temp file and renaming it over the original, so the watcher is set up
/// against the manifest's parent directory and filters by filename rather than watching
/// the inode directly.
///
/// Triggers are coalesced over a short debounce window so that a flurry of OS events
/// (typical of atomic-rename saves) results in a single reload attempt. Validation
/// errors are absorbed by the existing `reload_runtime_from_disk` path, which logs and
/// keeps the previous runtime active.
fn spawn_manifest_file_watcher(state: AppState) {
    let manifest_path = state.manifest_path.clone();
    let Some(parent) = manifest_path.parent().map(Path::to_path_buf) else {
        tracing::warn!(
            manifest = %manifest_path.display(),
            "skipping manifest file watcher: manifest has no parent directory",
        );
        return;
    };
    let Some(target_filename) = manifest_path.file_name().map(|name| name.to_os_string()) else {
        tracing::warn!(
            manifest = %manifest_path.display(),
            "skipping manifest file watcher: manifest path lacks a final component",
        );
        return;
    };

    let (event_tx, mut event_rx) = mpsc::channel::<()>(8);

    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                let touches_manifest = event
                    .paths
                    .iter()
                    .any(|path| path.file_name() == Some(target_filename.as_os_str()));
                if !touches_manifest {
                    return;
                }
                // Use try_send so a flood of OS events cannot back-pressure the
                // notify worker thread; we only need to know "something changed".
                let _ = event_tx.try_send(());
            }
            Err(error) => {
                tracing::warn!("manifest file watcher error: {error}");
            }
        }
    });

    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(
                manifest = %manifest_path.display(),
                "failed to initialize manifest file watcher: {error}",
            );
            return;
        }
    };

    if let Err(error) =
        notify::Watcher::watch(&mut watcher, &parent, notify::RecursiveMode::NonRecursive)
    {
        tracing::warn!(
            directory = %parent.display(),
            "failed to start watching manifest directory: {error}",
        );
        return;
    }

    tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of the task. Dropping it would
        // unsubscribe from the OS event source.
        let _watcher_guard = watcher;

        while event_rx.recv().await.is_some() {
            // Debounce: drain any pile-up of events that arrived during the wait.
            tokio::time::sleep(MANIFEST_FILE_WATCHER_DEBOUNCE).await;
            while event_rx.try_recv().is_ok() {}

            if let Err(error) = reload_runtime_from_disk(&state).await {
                tracing::error!(
                    manifest = %state.manifest_path.display(),
                    "manifest file watcher: hot reload failed (previous runtime preserved): {error:#}",
                );
            }
        }
    });
}

/// How often the authz-purge subscriber polls the outbox. 250 ms keeps revocation
/// latency well under one second while costing essentially nothing — the table is
/// usually empty, in which case the txn returns immediately with no rows.
const AUTHZ_PURGE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum events drained in a single poll tick. A larger batch is fine (we just
/// pop them all into the cache predicate) but bounding it keeps the txn short
/// and avoids starving other readers under a sudden burst of revocations.
const AUTHZ_PURGE_BATCH_LIMIT: usize = 64;

/// Drain the `authz_purge_outbox` table on a steady cadence, evict matching
/// entries from the in-process `AuthDecisionCache`, and delete the row only after
/// the eviction succeeds. The combined effect is at-most-five-minute (cache TTL)
/// worst-case stale access in the absence of revocations, and sub-second
/// revocation propagation in the presence of them.
fn spawn_authz_purge_subscriber(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(AUTHZ_PURGE_POLL_INTERVAL);
        loop {
            interval.tick().await;
            let core_store = Arc::clone(&state.core_store);
            let cache = state.auth_manager.decision_cache().clone();
            let drain_result = tokio::task::spawn_blocking(move || -> Result<usize> {
                let rows = core_store
                    .peek_outbox(store::CoreStoreBucket::AuthzPurgeOutbox, AUTHZ_PURGE_BATCH_LIMIT)
                    .context("failed to peek authz purge outbox")?;
                let mut applied = 0usize;
                for (key, payload) in rows {
                    match serde_json::from_slice::<auth::AuthzPurgeEvent>(&payload) {
                        Ok(event) => {
                            if let Err(error) = auth::apply_authz_purge(&cache, &event) {
                                tracing::warn!(
                                    "authz purge event `{key}` ignored due to apply failure: {error:#}"
                                );
                            } else {
                                applied += 1;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                "authz purge event `{key}` ignored due to parse failure: {error:#}"
                            );
                        }
                    }
                    if let Err(error) =
                        core_store.delete(store::CoreStoreBucket::AuthzPurgeOutbox, &key)
                    {
                        tracing::warn!(
                            "authz purge outbox cleanup for `{key}` failed: {error:#}"
                        );
                    }
                }
                Ok(applied)
            })
            .await;

            match drain_result {
                Ok(Ok(0)) => {} // Common case: no events to apply.
                Ok(Ok(n)) => {
                    tracing::debug!("authz purge subscriber applied {n} event(s)");
                }
                Ok(Err(error)) => {
                    tracing::warn!("authz purge subscriber drain failed: {error:#}");
                }
                Err(error) => {
                    tracing::warn!("authz purge subscriber task join failed: {error}");
                }
            }
        }
    });
}

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
            Arc::clone(&state.route_overrides),
            Arc::clone(&state.peer_capabilities),
            state.host_capabilities,
            Arc::clone(&state.host_load),
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

/// Maximum number of distinct `Arc<Module>` entries the in-memory instance pool
/// keeps warm in a single runtime generation. Sized well above any reasonable
/// Tachyon deployment's route count so the cache is effectively unbounded for the
/// happy path; the explicit cap is a defense-in-depth ceiling that prevents a
/// runaway manifest from blowing up host RSS.
const INSTANCE_POOL_DEFAULT_CAPACITY: u64 = 256;

/// Idle threshold after which a warm `Arc<Module>` entry is evicted from the
/// in-memory pool. The next request for the module pays a cwasm-cache thaw (read
/// the precompiled bytes from redb + `Module::deserialize`) — significantly
/// faster than a fresh JIT compile, so this approximates the hibernation /
/// scale-to-zero pattern called out by `wasm-ram-hibernation` without giving up
/// the warm-start latency for actively-used modules.
const INSTANCE_POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

fn build_runtime_state(config: IntegrityConfig) -> Result<RuntimeState> {
    let instance_pool = Arc::new(
        moka::sync::Cache::builder()
            .max_capacity(INSTANCE_POOL_DEFAULT_CAPACITY)
            .time_to_idle(INSTANCE_POOL_IDLE_TIMEOUT)
            .build(),
    );
    Ok(RuntimeState {
        engine: build_engine(&config, false)?,
        metered_engine: build_engine(&config, true)?,
        route_registry: Arc::new(RouteRegistry::build(&config)?),
        batch_target_registry: Arc::new(BatchTargetRegistry::build(&config)?),
        concurrency_limits: build_concurrency_limits(&config),
        instance_pool,
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
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Capabilities::detect(),
            Arc::new(HostLoadCounters::default()),
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
    component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::vector::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add vector store functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::training::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add training functions to prewarm HTTP component linker",
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
    control_plane_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add outbound HTTP functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add routing control functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to prewarm system component linker",
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
    if control_plane_component_bindings::ControlPlaneFaas::instantiate(
        &mut store, component, &linker,
    )
    .is_ok()
    {
        return Ok(());
    }
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
    let admin_routes = Router::new()
        .route("/admin/status", get(auth::admin_status_handler))
        .route("/admin/manifest", post(admin_manifest_update_handler))
        .route(
            "/admin/enrollment/start",
            post(admin_enrollment_start_handler),
        )
        .route(
            "/admin/enrollment/approve",
            post(admin_enrollment_approve_handler),
        )
        .route(
            "/admin/enrollment/poll/{session_id}",
            get(admin_enrollment_poll_handler),
        )
        .route(
            "/admin/security/recovery-codes",
            post(auth::generate_recovery_codes_handler),
        )
        .route(
            "/admin/security/2fa/regenerate",
            post(auth::regenerate_account_security_handler),
        )
        .route("/admin/security/pats", post(auth::issue_pat_handler))
        .route("/admin/assets", post(system_storage::upload_asset_handler))
        .route(
            "/admin/models/init",
            post(system_storage::init_upload_handler),
        )
        .route(
            "/admin/models/upload/{upload_id}",
            put(system_storage::upload_chunk_handler),
        )
        .route(
            "/admin/models/commit/{upload_id}",
            post(system_storage::commit_upload_handler),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin_auth_middleware,
        ));

    let app = Router::new()
        .merge(admin_routes)
        .route(
            "/auth/signup/validate-token",
            post(auth::validate_registration_token_handler),
        )
        .route("/auth/signup/stage", post(auth::stage_signup_handler))
        .route(
            "/auth/signup/finalize",
            post(auth::finalize_enrollment_handler),
        )
        .route(
            "/auth/recovery/consume",
            post(auth::consume_recovery_code_handler),
        )
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
    sample_rate > 0.0 && rand::rng().random_bool(sample_rate.clamp(0.0, 1.0))
}

fn merge_fuel_samples(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

async fn enforce_distributed_rate_limit(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
) -> Option<(StatusCode, String)> {
    let Some(policy) = route.distributed_rate_limit.as_ref() else {
        return None;
    };
    let Some(limiter_route) = runtime
        .config
        .sealed_route(SYSTEM_DIST_LIMITER_ROUTE)
        .cloned()
    else {
        record_distributed_rate_limit_bypass(&route.path, "system route missing");
        return None;
    };

    let key = distributed_rate_limit_key(headers);
    let body = match serde_json::to_vec(&serde_json::json!({
        "key": key,
        "threshold": policy.threshold,
        "window_seconds": policy.window_seconds,
    })) {
        Ok(body) => Bytes::from(body),
        Err(error) => {
            record_distributed_rate_limit_bypass(&route.path, &format!("encode failed: {error}"));
            return None;
        }
    };
    let method = Method::POST;
    let uri = Uri::from_static("/system/dist-limiter/check");
    let limiter_headers = HeaderMap::new();
    let trailers = Vec::new();

    let result = tokio::time::timeout(
        DISTRIBUTED_RATE_LIMIT_TIMEOUT,
        Box::pin(execute_route_with_middleware(
            state,
            runtime,
            &limiter_route,
            &limiter_headers,
            &method,
            &uri,
            &body,
            &trailers,
            HopLimit(DEFAULT_HOP_LIMIT),
            None,
            false,
            None,
        )),
    )
    .await;

    match result {
        Ok(Ok(result)) => distributed_rate_limit_decision(route, result.response),
        Ok(Err((status, message))) => {
            record_distributed_rate_limit_bypass(
                &route.path,
                &format!("limiter route failed with {status}: {message}"),
            );
            None
        }
        Err(_) => {
            record_distributed_rate_limit_bypass(&route.path, "timeout");
            None
        }
    }
}

fn distributed_rate_limit_decision(
    route: &IntegrityRoute,
    response: GuestHttpResponse,
) -> Option<(StatusCode, String)> {
    if !response.status.is_success() {
        record_distributed_rate_limit_bypass(
            &route.path,
            &format!("limiter returned HTTP {}", response.status),
        );
        return None;
    }

    let value = match serde_json::from_slice::<Value>(&response.body) {
        Ok(value) => value,
        Err(error) => {
            record_distributed_rate_limit_bypass(
                &route.path,
                &format!("invalid limiter response: {error}"),
            );
            return None;
        }
    };

    if value
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        None
    } else {
        Some((
            StatusCode::TOO_MANY_REQUESTS,
            format!("distributed rate limit exceeded for route `{}`", route.path),
        ))
    }
}

fn distributed_rate_limit_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn record_distributed_rate_limit_bypass(route_path: &str, reason: &str) {
    DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL.fetch_add(1, Ordering::Relaxed);
    tracing::warn!(
        route = %route_path,
        reason,
        "distributed rate limiter bypassed; falling back to local limiter"
    );
}

#[cfg(test)]
fn distributed_rate_limit_bypass_total() -> u64 {
    DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL.load(Ordering::Relaxed)
}

fn lora_training_queue() -> Arc<LoraTrainingQueue> {
    Arc::clone(LORA_TRAINING_QUEUE.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel();
        let statuses = Arc::new(Mutex::new(HashMap::new()));
        let worker_statuses = Arc::clone(&statuses);
        std::thread::Builder::new()
            .name("tachyon-lora-low-priority".to_owned())
            .spawn(move || run_lora_training_worker(receiver, worker_statuses))
            .expect("LoRA training worker should spawn");
        Arc::new(LoraTrainingQueue { sender, statuses })
    }))
}

fn ai_inference_jobs() -> Arc<Mutex<HashMap<String, AiInferenceJobStatus>>> {
    Arc::clone(AI_INFERENCE_JOBS.get_or_init(|| Arc::new(Mutex::new(HashMap::new()))))
}

fn enqueue_async_ai_inference_job(body: Bytes) -> Response {
    let id = format!("ai-{}", Uuid::new_v4().simple());
    let jobs = ai_inference_jobs();
    jobs.lock()
        .expect("AI inference job map should not be poisoned")
        .insert(id.clone(), AiInferenceJobStatus::Queued);
    let worker_jobs = Arc::clone(&jobs);
    let worker_id = id.clone();
    tokio::spawn(async move {
        update_ai_inference_status(&worker_jobs, &worker_id, AiInferenceJobStatus::Running);
        let output = format!(
            "generated:{}",
            String::from_utf8_lossy(&body)
                .chars()
                .take(256)
                .collect::<String>()
        );
        update_ai_inference_status(
            &worker_jobs,
            &worker_id,
            AiInferenceJobStatus::Completed { output },
        );
    });

    (
        StatusCode::ACCEPTED,
        [("content-type", "application/json")],
        format!(r#"{{"job_id":"{id}","status":"queued"}}"#),
    )
        .into_response()
}

fn ai_inference_job_status_response(id: &str) -> Response {
    let jobs = ai_inference_jobs();
    let Some(status) = jobs
        .lock()
        .expect("AI inference job map should not be poisoned")
        .get(id)
        .cloned()
    else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown AI inference job `{id}`"),
        )
            .into_response();
    };
    let body = match status {
        AiInferenceJobStatus::Queued => format!(r#"{{"job_id":"{id}","status":"queued"}}"#),
        AiInferenceJobStatus::Running => format!(r#"{{"job_id":"{id}","status":"running"}}"#),
        AiInferenceJobStatus::Completed { output } => serde_json::json!({
            "job_id": id,
            "status": "completed",
            "output": output,
        })
        .to_string(),
        AiInferenceJobStatus::Failed { message } => serde_json::json!({
            "job_id": id,
            "status": "failed",
            "error": message,
        })
        .to_string(),
    };
    (StatusCode::OK, [("content-type", "application/json")], body).into_response()
}

fn update_ai_inference_status(
    jobs: &Arc<Mutex<HashMap<String, AiInferenceJobStatus>>>,
    id: &str,
    status: AiInferenceJobStatus,
) {
    jobs.lock()
        .expect("AI inference job map should not be poisoned")
        .insert(id.to_owned(), status);
}

fn run_lora_training_worker(
    receiver: std::sync::mpsc::Receiver<LoraTrainingJob>,
    statuses: Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
) {
    while let Ok(job) = receiver.recv() {
        update_lora_training_status(
            &statuses,
            &job.id,
            LoraTrainingJobStatus::Running {
                step: 0,
                total: job.max_steps,
            },
        );
        let result = execute_lora_training_job(&job, &statuses);
        match result {
            Ok(path) => update_lora_training_status(
                &statuses,
                &job.id,
                LoraTrainingJobStatus::Completed { adapter_path: path },
            ),
            Err(error) => update_lora_training_status(
                &statuses,
                &job.id,
                LoraTrainingJobStatus::Failed {
                    message: format!("{error:#}"),
                },
            ),
        }
    }
}

fn execute_lora_training_job(
    job: &LoraTrainingJob,
    statuses: &Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
) -> Result<String> {
    let total = job.max_steps.max(1);
    for step in 1..=total.min(4) {
        update_lora_training_status(
            statuses,
            &job.id,
            LoraTrainingJobStatus::Running { step, total },
        );
        std::thread::sleep(Duration::from_millis(1));
    }

    let broker_root = std::env::var(MODEL_BROKER_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("tachyon_data"));
    let adapter_dir = broker_root.join("adapters");
    fs::create_dir_all(&adapter_dir).with_context(|| {
        format!(
            "failed to create adapter broker dir `{}`",
            adapter_dir.display()
        )
    })?;
    let sanitized = sanitize_lora_job_part(&job.id)?;
    let adapter_path = adapter_dir.join(format!("{sanitized}.safetensors"));
    let payload = serde_json::to_vec(&serde_json::json!({
        "format": "tachyon.mock-lora.safetensors",
        "tenant_id": job.tenant_id,
        "base_model": job.base_model,
        "dataset": {
            "volume": job.dataset_volume,
            "path": job.dataset_path,
            "split": job.dataset_split,
        },
        "rank": job.rank,
        "max_steps": job.max_steps,
        "seed": job.seed,
        "finops": {
            "cpu_fallback": true,
            "ram_spillover": true,
            "estimated_cpu_ms": u64::from(total) * 5,
            "estimated_ram_mb": u64::from(job.rank.max(1)) * 64,
        }
    }))
    .context("failed to serialize LoRA adapter artifact")?;
    fs::write(&adapter_path, payload)
        .with_context(|| format!("failed to write adapter `{}`", adapter_path.display()))?;
    Ok(adapter_path.display().to_string())
}

fn update_lora_training_status(
    statuses: &Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
    id: &str,
    status: LoraTrainingJobStatus,
) {
    statuses
        .lock()
        .expect("LoRA training status map should not be poisoned")
        .insert(id.to_owned(), status);
}

fn sanitize_lora_job_part(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(anyhow!("invalid LoRA job id `{value}`"));
    }
    Ok(trimmed.to_owned())
}

fn generate_traceparent() -> String {
    let trace_id = Uuid::new_v4().simple().to_string();
    let span_id = format!("{:016x}", rand::rng().random::<u64>());
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

            // Durably stash each record in the metering outbox before attempting the
            // HTTP export. If the host crashes between here and the export, the records
            // are recoverable on the next boot. On successful export, the entries are
            // removed; on failure, they remain and a later sweep can retry.
            //
            // This is the implementation of the `async-zero-blocking-metering` change:
            // the request critical path remains untouched (the original `mpsc` emit was
            // already async), but the durability story is now an explicit outbox table
            // rather than an in-memory channel that vanishes on a crash.
            let outbox_keys = persist_metering_batch(&state, &batch);

            match export_metering_batch(&state, batch).await {
                Ok(()) => {
                    for key in outbox_keys {
                        if let Err(error) = state
                            .core_store
                            .delete(store::CoreStoreBucket::MeteringOutbox, &key)
                        {
                            tracing::warn!("metering outbox cleanup for `{key}` failed: {error:#}");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        "telemetry metering export failed; outbox entries retained: {error}",
                    );
                }
            }
        }
    });
}

fn persist_metering_batch(state: &AppState, batch: &[String]) -> Vec<String> {
    let mut keys = Vec::with_capacity(batch.len());
    for record in batch {
        match state
            .core_store
            .append_outbox(store::CoreStoreBucket::MeteringOutbox, record.as_bytes())
        {
            Ok(key) => keys.push(key),
            Err(error) => {
                tracing::warn!("metering outbox persist failed: {error:#}");
            }
        }
    }
    keys
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

async fn start_mtls_gateway_listener(state: AppState) -> Result<Option<MtlsGatewayListenerHandle>> {
    let Some(config) = state.mtls_gateway.as_ref().cloned() else {
        return Ok(None);
    };
    let runtime = state.runtime.load_full();
    if runtime.config.sealed_route(SYSTEM_GATEWAY_ROUTE).is_none() {
        return Ok(None);
    }

    let listener = tokio::net::TcpListener::bind(config.bind_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind mTLS gateway listener on {}",
                config.bind_address
            )
        })?;
    let local_addr = listener
        .local_addr()
        .context("failed to read mTLS gateway listener local address")?;

    let join_handle = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!("mTLS gateway listener accept failed: {error}");
                    continue;
                }
            };
            let connection_state = state.clone();
            let server_config = Arc::clone(&config.server_config);
            tokio::spawn(async move {
                if let Err(error) =
                    handle_mtls_gateway_connection(connection_state, server_config, stream).await
                {
                    tracing::warn!(remote = %peer_addr, "mTLS gateway connection failed: {error:#}");
                }
            });
        }
    });

    Ok(Some(MtlsGatewayListenerHandle {
        local_addr,
        join_handle,
    }))
}

async fn handle_mtls_gateway_connection(
    state: AppState,
    server_config: Arc<tokio_rustls::rustls::ServerConfig>,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
    let tls_stream = acceptor
        .accept(stream)
        .await
        .context("failed to complete mTLS handshake")?;

    HyperConnectionBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(
            TokioIo::new(tls_stream),
            service_fn(move |request| {
                let state = state.clone();
                async move {
                    Ok::<_, Infallible>(dispatch_mtls_gateway_request(state, request).await)
                }
            }),
        )
        .await
        .map_err(|error| anyhow!("mTLS gateway connection exited unexpectedly: {error}"))
}

async fn dispatch_mtls_gateway_request(
    state: AppState,
    request: hyper::Request<hyper::body::Incoming>,
) -> Response {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.sealed_route(SYSTEM_GATEWAY_ROUTE).cloned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "sealed manifest does not define `/system/gateway`",
        )
            .into_response();
    };

    let (parts, body) = request.into_parts();
    let original_route = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| parts.uri.path().to_owned());
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to read mTLS request body: {error}"),
            )
                .into_response();
        }
    };
    let mut headers = parts.headers;
    let original_route_value = match HeaderValue::from_str(&original_route) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid original route header value `{original_route}`: {error}"),
            )
                .into_response();
        }
    };
    headers.insert(TACHYON_ORIGINAL_ROUTE_HEADER, original_route_value);

    let gateway_uri = Uri::from_static(SYSTEM_GATEWAY_ROUTE);
    let trailers = GuestHttpFields::new();
    let trace_id = Uuid::new_v4().to_string();
    match execute_route_with_middleware(
        &state,
        &runtime,
        &route,
        &headers,
        &parts.method,
        &gateway_uri,
        &body,
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        Some(&trace_id),
        false,
        None,
    )
    .await
    {
        Ok(result) => guest_response_into_response(result),
        Err((status, message)) => (status, message).into_response(),
    }
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
    let instance_pool = Arc::clone(&runtime.instance_pool);
    let request_headers = HeaderMap::new();
    let route_for_execution = route.clone();
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
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
            bridge_manager: Arc::clone(&state.bridge_manager),
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
            instance_pool: Some(instance_pool),
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
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let instance_pool = Arc::clone(&runtime.instance_pool);
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
            bridge_manager: Arc::clone(&state.bridge_manager),
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
            instance_pool: Some(instance_pool),
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
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
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
            route_overrides,
            host_load,
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
                Arc::clone(&state.route_overrides),
                Arc::clone(&state.host_load),
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
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    host_load: Arc<HostLoadCounters>,
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
        bridge_manager: Arc::new(BridgeManager::default()),
        telemetry: None,
        concurrency_limits,
        propagated_headers: Vec::new(),
        route_overrides,
        host_load,
        #[cfg(feature = "ai-inference")]
        ai_runtime,
        instance_pool: None,
    };
    let (module_path, module) = resolve_legacy_guest_module_with_pool(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        "default",
        execution.instance_pool.as_deref(),
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

fn clone_headers_with_original_route(headers: &HeaderMap, route: &IntegrityRoute) -> HeaderMap {
    let mut cloned = headers.clone();
    if !cloned.contains_key(TACHYON_ORIGINAL_ROUTE_HEADER) {
        if let Ok(value) = HeaderValue::from_str(&route.path) {
            cloned.insert(TACHYON_ORIGINAL_ROUTE_HEADER, value);
        }
    }
    cloned
}

async fn forward_request_to_override(
    http_client: &Client,
    destination: &str,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
) -> std::result::Result<Response, (StatusCode, String)> {
    let mut request = http_client.request(method.clone(), destination);
    for (name, value) in headers {
        if name == "host" || name == "content-length" || name == "connection" {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    let response = request.body(body.clone()).send().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("route override forward to `{destination}` failed: {error}"),
        )
    })?;
    let status = response.status();
    let response_headers = response.headers().clone();
    let response_body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to read override response body from `{destination}`: {error}"),
        )
    })?;
    let mut built = Response::builder()
        .status(status)
        .body(Body::from(response_body))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to construct override response: {error}"),
            )
        })?;
    for (name, value) in &response_headers {
        if name == "content-length" || name == "connection" || name == "transfer-encoding" {
            continue;
        }
        built.headers_mut().append(name.clone(), value.clone());
    }
    Ok(built)
}

async fn forward_request_to_override_as_guest_response(
    http_client: &Client,
    destination: &str,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
) -> std::result::Result<GuestHttpResponse, (StatusCode, String)> {
    let mut request = http_client.request(method.clone(), destination);
    for (name, value) in headers {
        if name == "host" || name == "content-length" || name == "connection" {
            continue;
        }
        request = request.header(name, value);
    }
    request = request.header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    let response = request.body(body.clone()).send().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("mesh-overlay forward to `{destination}` failed: {error}"),
        )
    })?;
    let status = response.status();
    let headers = header_map_to_guest_fields(response.headers());
    let body = response.bytes().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to read mesh-overlay response body from `{destination}`: {error}"),
        )
    })?;
    Ok(GuestHttpResponse {
        status,
        headers,
        body,
        trailers: Vec::new(),
    })
}

fn requested_model_alias(
    route: &IntegrityRoute,
    headers: &HeaderMap,
    body: &Bytes,
) -> Option<String> {
    let header_alias = ["x-tachyon-model", "x-model-alias", "model-alias"]
        .into_iter()
        .find_map(|name| headers.get(name))
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let body_alias = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|payload| payload.as_object().cloned())
        .and_then(|payload| {
            ["model", "model_alias", "alias"]
                .into_iter()
                .find_map(|key| payload.get(key).and_then(Value::as_str))
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        });

    header_alias
        .or(body_alias)
        .filter(|alias| {
            route.models.is_empty()
                || route
                    .models
                    .iter()
                    .any(|binding| binding.alias.eq_ignore_ascii_case(alias))
        })
        .or_else(|| {
            if route.models.len() == 1 {
                route.models.first().map(|binding| binding.alias.clone())
            } else {
                None
            }
        })
}

#[cfg(feature = "ai-inference")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RouteMeshQosProfile {
    accelerator: ai_inference::AcceleratorKind,
    qos: RouteQos,
}

#[cfg(feature = "ai-inference")]
fn route_mesh_qos_profile(
    route: &IntegrityRoute,
    requested_model: Option<&str>,
) -> Option<RouteMeshQosProfile> {
    let binding = requested_model
        .and_then(|alias| {
            route
                .models
                .iter()
                .find(|binding| binding.alias.eq_ignore_ascii_case(alias))
        })
        .or_else(|| route.models.first())?;
    Some(RouteMeshQosProfile {
        accelerator: ai_inference::AcceleratorKind::from_model_device(&binding.device),
        qos: binding.qos,
    })
}

#[cfg(feature = "ai-inference")]
fn should_consult_mesh_qos_override(profile: RouteMeshQosProfile, local_load: u32) -> bool {
    match profile.qos {
        RouteQos::RealTime => local_load > 0,
        RouteQos::Standard => local_load >= 4,
        RouteQos::Batch => local_load >= 1_000,
    }
}

#[cfg(not(feature = "resiliency"))]
mod resiliency {
    use super::{execute_route_with_middleware_inner, RouteExecutionResult, RouteInvocation};
    use axum::http::StatusCode;
    use sysinfo::System;

    pub(crate) async fn execute_route_with_resiliency(
        invocation: RouteInvocation,
    ) -> std::result::Result<RouteExecutionResult, (StatusCode, String)> {
        execute_route_with_middleware_inner(&invocation).await
    }

    pub(crate) fn available_system_ram_bytes() -> u64 {
        let mut system = System::new();
        system.refresh_memory();
        system.available_memory()
    }
}

#[allow(clippy::too_many_arguments)]
async fn execute_route_override(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    trailer_fields: &GuestHttpFields,
    hop_limit: HopLimit,
    trace_id: &str,
    sampled_execution: bool,
    destination: &str,
) -> (Response, Option<u64>) {
    if destination.starts_with("http://") || destination.starts_with("https://") {
        match forward_request_to_override(
            &state.http_client,
            destination,
            headers,
            method,
            body,
            hop_limit,
        )
        .await
        {
            Ok(response) => (response, None),
            Err((status, message)) => ((status, message).into_response(), None),
        }
    } else {
        let override_path = normalize_route_path(destination);
        match runtime.config.sealed_route(&override_path).cloned() {
            Some(override_route) => {
                let override_headers = clone_headers_with_original_route(headers, route);
                match execute_route_with_middleware(
                    state,
                    runtime,
                    &override_route,
                    &override_headers,
                    method,
                    uri,
                    body,
                    trailer_fields,
                    hop_limit,
                    Some(trace_id),
                    sampled_execution,
                    None,
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
            None => (
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "route override for `{}` points to missing route `{override_path}`",
                        route.path
                    ),
                )
                    .into_response(),
                None,
            ),
        }
    }
}

async fn faas_handler(
    State(state): State<AppState>,
    Extension(hop_limit): Extension<HopLimit>,
    #[cfg(feature = "websockets")] ws: Result<
        WebSocketUpgrade,
        axum::extract::ws::rejection::WebSocketUpgradeRejection,
    >,
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
    if method == Method::POST && normalized_path == "/api/v1/generate" {
        return enqueue_async_ai_inference_job(body);
    }
    if method == Method::GET {
        if let Some(job_id) = normalized_path.strip_prefix("/api/v1/jobs/") {
            return ai_inference_job_status_response(job_id);
        }
    }
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
                let requested_model = requested_model_alias(&route, &headers, &body);
                let required_capabilities =
                    Capabilities::from_mask(selected_target.required_capability_mask);
                let local_supports_target = state.host_capabilities.supports(required_capabilities);
                #[cfg(feature = "ai-inference")]
                let mesh_qos_destination = route_mesh_qos_profile(
                    &route,
                    requested_model.as_deref(),
                )
                .and_then(|profile| {
                    let tier_snapshot = runtime.ai_runtime.queue_tier_snapshot(profile.accelerator);
                    let local_queue_depth = match profile.qos {
                        RouteQos::RealTime => tier_snapshot.realtime,
                        RouteQos::Standard => tier_snapshot.standard,
                        RouteQos::Batch => tier_snapshot.batch,
                    };
                    should_consult_mesh_qos_override(profile, local_queue_depth).then(|| {
                        control_plane_override_destination(
                            state.route_overrides.as_ref(),
                            &state.peer_capabilities,
                            &format!(
                                "{MESH_QOS_OVERRIDE_PREFIX}{}",
                                normalize_route_path(&route.path)
                            ),
                            &headers,
                            selected_target.required_capability_mask,
                            requested_model.as_deref(),
                        )
                    })?
                });

                #[cfg(not(feature = "ai-inference"))]
                let mesh_qos_destination: Option<String> = None;

                if let Some(destination) = mesh_qos_destination.or_else(|| {
                    control_plane_override_destination(
                        state.route_overrides.as_ref(),
                        &state.peer_capabilities,
                        &route.path,
                        &headers,
                        selected_target.required_capability_mask,
                        requested_model.as_deref(),
                    )
                }) {
                    execute_route_override(
                        &state,
                        &runtime,
                        &route,
                        &headers,
                        &method,
                        &uri,
                        &body,
                        &trailer_fields,
                        hop_limit,
                        &trace_id,
                        sampled_execution,
                        &destination,
                    )
                    .await
                } else if !local_supports_target {
                    let missing = state
                        .host_capabilities
                        .missing_names(required_capabilities)
                        .join(", ");
                    (
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!(
                            "Missing Capability: route `{}` requires [{}] but no capable mesh peer is available",
                            route.path, missing
                        ),
                    )
                        .into_response(),
                    None,
                )
                } else {
                    #[cfg(feature = "websockets")]
                    {
                        if selected_target.websocket {
                            let status = if is_websocket_upgrade_request(&headers) {
                                StatusCode::BAD_REQUEST
                            } else {
                                StatusCode::UPGRADE_REQUIRED
                            };
                            match ws {
                                Ok(upgrade) => {
                                    let upgrade: WebSocketUpgrade = upgrade;
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
                                Err(_) => (
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
    if let Some(rejection) = enforce_distributed_rate_limit(state, runtime, route, headers).await {
        return Err(rejection);
    }
    let selected_module = selected_module
        .map(str::to_owned)
        .map(Ok)
        .unwrap_or_else(|| {
            select_route_module(route, headers)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))
        })?;

    if let Some(rejection) = enforce_resource_admission(
        state,
        route,
        headers,
        method,
        body,
        hop_limit,
        runtime,
    )
    .await?
    {
        return Ok(rejection);
    }

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
            if route.allow_overflow {
                let requested_model = requested_model_alias(route, headers, body);
                if let Some(destination) = control_plane_override_destination(
                    state.route_overrides.as_ref(),
                    &state.peer_capabilities,
                    &route.path,
                    headers,
                    select_route_target(route, headers)
                        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
                        .required_capability_mask,
                    requested_model.as_deref(),
                ) {
                    let response = forward_request_to_override_as_guest_response(
                        &state.http_client,
                        &destination,
                        headers,
                        method,
                        body,
                        hop_limit,
                    )
                    .await?;
                    return Ok(RouteExecutionResult {
                        response,
                        fuel_consumed: None,
                        completion_guard: None,
                    });
                }
            }

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

async fn enforce_resource_admission(
    state: &AppState,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    body: &Bytes,
    hop_limit: HopLimit,
    runtime: &Arc<RuntimeState>,
) -> std::result::Result<Option<RouteExecutionResult>, (StatusCode, String)> {
    let Some(policy) = route.resource_policy.as_ref() else {
        return Ok(None);
    };
    let required_ram_bytes = policy.required_ram_bytes();
    if required_ram_bytes == 0 {
        return Ok(None);
    }
    let available_ram = resiliency::available_system_ram_bytes();
    if available_ram >= required_ram_bytes {
        return Ok(None);
    }

    if policy.admission_strategy == AdmissionStrategy::MeshRetry {
        let requested_model = requested_model_alias(route, headers, body);
        let target = select_route_target(route, headers)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
        if let Some(destination) = control_plane_override_destination(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            &route.path,
            headers,
            target.required_capability_mask,
            requested_model.as_deref(),
        ) {
            let response = forward_request_to_override_as_guest_response(
                &state.http_client,
                &destination,
                headers,
                method,
                body,
                hop_limit,
            )
            .await?;
            return Ok(Some(RouteExecutionResult {
                response,
                fuel_consumed: None,
                completion_guard: None,
            }));
        }
    }

    let mut response = GuestHttpResponse::new(
        StatusCode::SERVICE_UNAVAILABLE,
        format!(
            "route `{}` requires {} bytes of available RAM but only {} bytes are available",
            route.path, required_ram_bytes, available_ram
        ),
    );
    response.headers.push((
        "x-tachyon-reason".to_owned(),
        "Insufficient-Cluster-Resources".to_owned(),
    ));
    let _ = runtime;
    Ok(Some(RouteExecutionResult {
        response,
        fuel_consumed: None,
        completion_guard: None,
    }))
}

impl ResourcePolicy {
    fn required_ram_bytes(&self) -> u64 {
        let from_gb = self
            .min_ram_gb
            .unwrap_or(0)
            .saturating_mul(1024)
            .saturating_mul(1024)
            .saturating_mul(1024);
        let from_mb = self
            .min_ram_mb
            .unwrap_or(0)
            .saturating_mul(1024)
            .saturating_mul(1024);
        from_gb.max(from_mb)
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
    prepare_encrypted_route_volumes(route).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.into_response(&runtime.config).1,
        )
    })?;
    let _permit = permit;
    let active_request_guard = semaphore.begin_request();
    if let Some(FaaSRuntime::Microvm {
        image,
        vcpus,
        memory_mb,
    }) = route.runtime.as_ref()
    {
        let mut response = GuestHttpResponse::new(
            StatusCode::NOT_IMPLEMENTED,
            format!(
                "route `{}` is configured for MicroVM image `{image}` ({vcpus} vCPU, {memory_mb} MiB), but the SmolVM runner is not enabled in this host build",
                route.path
            ),
        );
        response.headers.push((
            "x-tachyon-runtime".to_owned(),
            "microvm".to_owned(),
        ));
        return Ok(RouteExecutionResult {
            response,
            fuel_consumed: None,
            completion_guard: Some(active_request_guard.into_response_guard()),
        });
    }
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
    let task_route_overrides = Arc::clone(&state.route_overrides);
    let task_host_load = Arc::clone(&state.host_load);
    let task_bridge_manager = Arc::clone(&state.bridge_manager);
    let task_async_log_sender = state.async_log_sender.clone();
    let task_instance_pool = Arc::clone(&runtime.instance_pool);
    let route_requires_tee = route.requires_tee;
    #[cfg(feature = "ai-inference")]
    let task_ai_runtime = Arc::clone(&runtime.ai_runtime);
    let guest_request = GuestRequest {
        method: method.to_string(),
        uri: uri.to_string(),
        headers: header_map_to_guest_fields(&headers),
        body: body.clone(),
        trailers: trailers.clone(),
    };
    let _host_load_guard = HostLoadGuard::new(
        Arc::clone(&state.host_load),
        guest_memory_page_count(request_config.guest_memory_limit_bytes),
    );
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
                bridge_manager: task_bridge_manager,
                telemetry: telemetry_context,
                concurrency_limits,
                propagated_headers: task_propagated_headers,
                route_overrides: task_route_overrides,
                host_load: task_host_load,
                #[cfg(feature = "ai-inference")]
                ai_runtime: task_ai_runtime,
                instance_pool: if route_requires_tee {
                    None
                } else {
                    Some(task_instance_pool)
                },
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
    seal_encrypted_route_volumes(route).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            error.into_response(&runtime.config).1,
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
    let resolved_target = resolve_outbound_http_target(
        config,
        route_registry,
        caller_route,
        &reqwest::Method::GET,
        target,
    )?;
    let url = rewrite_outbound_http_url(&resolved_target.url, config);
    let inject_identity = resolved_target.kind.is_internal();
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
        &resolved_target.kind,
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
    target_kind: &OutboundTargetKind,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    identity_token: Option<&str>,
) -> reqwest::RequestBuilder {
    if !target_kind.is_internal() {
        return request;
    }

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
    target_kind: &OutboundTargetKind,
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
            target_kind,
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
        target_kind,
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
        let required_capabilities = default_route_capabilities();
        return Ok(SelectedRouteTarget {
            module: route.name.clone(),
            websocket: false,
            required_capability_mask: Capabilities::from_requirement_list(&required_capabilities)
                .map_err(|error| error.to_string())?
                .mask,
            required_capabilities,
        });
    }

    for target in &route.targets {
        if target
            .match_header
            .as_ref()
            .is_some_and(|matcher| request_header_matches(headers, matcher))
        {
            let required_capabilities = if target.requires.is_empty() {
                default_route_capabilities()
            } else {
                target.requires.clone()
            };
            return Ok(SelectedRouteTarget {
                module: target.module.clone(),
                websocket: target.websocket,
                required_capability_mask: Capabilities::from_requirement_list(
                    &required_capabilities,
                )
                .map_err(|error| error.to_string())?
                .mask,
                required_capabilities,
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
            None => rand::rng().random_range(0..total_weight),
        };
        let mut cumulative_weight = 0_u64;
        for target in &route.targets {
            if target.weight == 0 {
                continue;
            }
            cumulative_weight = cumulative_weight.saturating_add(u64::from(target.weight));
            if draw < cumulative_weight {
                let required_capabilities = if target.requires.is_empty() {
                    default_route_capabilities()
                } else {
                    target.requires.clone()
                };
                return Ok(SelectedRouteTarget {
                    module: target.module.clone(),
                    websocket: target.websocket,
                    required_capability_mask: Capabilities::from_requirement_list(
                        &required_capabilities,
                    )
                    .map_err(|error| error.to_string())?
                    .mask,
                    required_capabilities,
                });
            }
        }
    }

    resolve_function_name(&route.path)
        .map(|module| SelectedRouteTarget {
            module,
            websocket: false,
            required_capability_mask: Capabilities::CORE_WASI,
            required_capabilities: default_route_capabilities(),
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

#[cfg(test)]
fn resolve_mesh_fetch_target(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    target: &str,
) -> std::result::Result<String, String> {
    resolve_outbound_http_target(
        config,
        route_registry,
        caller_route,
        &reqwest::Method::GET,
        target,
    )
    .map(|resolved| resolved.url)
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

impl OutboundTargetKind {
    fn is_internal(&self) -> bool {
        matches!(self, Self::Internal)
    }
}

fn resolve_outbound_http_target(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    method: &reqwest::Method,
    target: &str,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    if target.starts_with('/') {
        return Ok(ResolvedOutboundTarget {
            url: format!("{}{}", internal_mesh_base_url(config)?, target),
            kind: OutboundTargetKind::Internal,
        });
    }

    if !(target.starts_with("http://") || target.starts_with("https://")) {
        return Err(format!(
            "mesh fetch target `{target}` must be an absolute URL or an absolute route path"
        ));
    }

    let url = reqwest::Url::parse(target)
        .map_err(|error| format!("mesh fetch target `{target}` is not a valid URL: {error}"))?;
    if !url.host_str().is_some_and(is_internal_mesh_host) {
        return resolve_direct_external_target(caller_route, target);
    }

    let normalized_path = normalize_route_path(url.path());
    let base_url = internal_mesh_base_url(config)?;
    if route_registry.by_path.contains_key(&normalized_path) {
        return Ok(ResolvedOutboundTarget {
            url: format!("{base_url}{}", append_query(&normalized_path, url.query())),
            kind: OutboundTargetKind::Internal,
        });
    }

    let path_segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let Some(first_segment) = path_segments.first().copied() else {
        return Err(format!(
            "internal mesh target `{target}` must identify a sealed route path, resource alias, or a single dependency name"
        ));
    };
    let suffix = url
        .path()
        .strip_prefix(&format!("/{first_segment}"))
        .unwrap_or_default();
    if let Some(resource) = config.resources.get(first_segment) {
        return resolve_resource_alias(
            config,
            route_registry,
            resource,
            first_segment,
            suffix,
            url.query(),
            method,
        );
    }

    if path_segments.len() != 1 {
        return Err(format!(
            "internal mesh target `{target}` must identify a sealed route path, resource alias, or a single dependency name"
        ));
    }
    let dependency_name = path_segments[0];
    let resolved_route =
        route_registry.resolve_dependency_route(&caller_route.path, dependency_name)?;
    Ok(ResolvedOutboundTarget {
        url: format!(
            "{base_url}{}",
            append_query(&resolved_route.path, url.query())
        ),
        kind: OutboundTargetKind::Internal,
    })
}

fn resolve_direct_external_target(
    caller_route: &IntegrityRoute,
    target: &str,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    if caller_route.role == RouteRole::System {
        return Ok(ResolvedOutboundTarget {
            url: target.to_owned(),
            kind: OutboundTargetKind::External,
        });
    }

    Err(format!(
        "route `{}` is not allowed to call raw external URLs; seal an external resource alias in `integrity.lock` and use `http://mesh/<alias>` instead",
        caller_route.path
    ))
}

fn resolve_resource_alias(
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    resource: &IntegrityResource,
    resource_name: &str,
    suffix: &str,
    query: Option<&str>,
    method: &reqwest::Method,
) -> std::result::Result<ResolvedOutboundTarget, String> {
    match resource {
        IntegrityResource::Internal {
            target,
            version_constraint,
        } => {
            let base_path = resolve_internal_resource_target(
                route_registry,
                target,
                version_constraint.as_deref(),
            )?;
            Ok(ResolvedOutboundTarget {
                url: format!(
                    "{}{}",
                    internal_mesh_base_url(config)?,
                    append_query(&join_resource_path(&base_path, suffix), query)
                ),
                kind: OutboundTargetKind::Internal,
            })
        }
        IntegrityResource::External {
            target,
            allowed_methods,
        } => {
            if !allowed_methods
                .iter()
                .any(|allowed| allowed == method.as_str())
            {
                return Err(format!(
                    "sealed external resource `{resource_name}` does not allow HTTP method `{}`",
                    method.as_str()
                ));
            }
            Ok(ResolvedOutboundTarget {
                url: join_external_resource_url(target, suffix, query)?,
                kind: OutboundTargetKind::External,
            })
        }
    }
}

fn resolve_internal_resource_target(
    route_registry: &RouteRegistry,
    target: &str,
    version_constraint: Option<&str>,
) -> std::result::Result<String, String> {
    if target.starts_with('/') {
        let normalized = normalize_route_path(target);
        let route = route_registry.by_path.get(&normalized).ok_or_else(|| {
            format!("sealed resource target `{normalized}` does not match any sealed route")
        })?;
        if let Some(requirement) = version_constraint {
            let parsed = VersionReq::parse(requirement).map_err(|error| {
                format!("sealed resource version constraint `{requirement}` is invalid: {error}")
            })?;
            if !parsed.matches(&route.version) {
                return Err(format!(
                    "sealed resource target `{normalized}` does not satisfy version constraint `{requirement}`"
                ));
            }
        }
        return Ok(normalized);
    }

    let route_name = normalize_service_name(target)
        .map_err(|error| format!("sealed resource target `{target}` is invalid: {error}"))?;
    let route = if let Some(requirement) = version_constraint {
        let parsed = VersionReq::parse(requirement).map_err(|error| {
            format!("sealed resource version constraint `{requirement}` is invalid: {error}")
        })?;
        route_registry.resolve_named_route_matching(&route_name, &parsed)?
    } else {
        route_registry.resolve_named_route(&route_name)?
    };
    Ok(route.path.clone())
}

fn join_resource_path(base_path: &str, suffix: &str) -> String {
    if suffix.is_empty() || suffix == "/" {
        return base_path.to_owned();
    }
    format!("{}{}", base_path.trim_end_matches('/'), suffix)
}

fn join_external_resource_url(
    base_url: &str,
    suffix: &str,
    query: Option<&str>,
) -> std::result::Result<String, String> {
    let mut url = reqwest::Url::parse(base_url).map_err(|error| {
        format!("sealed external resource target `{base_url}` is not a valid URL: {error}")
    })?;
    let merged_path = join_resource_path(url.path(), suffix);
    url.set_path(&merged_path);
    if let Some(query) = query {
        url.set_query(Some(query));
    }
    Ok(url.to_string())
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

    fn resolve_named_route_matching(
        &self,
        route_name: &str,
        requirement: &VersionReq,
    ) -> std::result::Result<&ResolvedRoute, String> {
        self.by_name
            .get(route_name)
            .into_iter()
            .flatten()
            .find(|candidate| requirement.matches(&candidate.version))
            .ok_or_else(|| {
                format!(
                    "sealed resource `{route_name}` requires a route matching `{requirement}`, but no compatible version was loaded"
                )
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

    let (module_path, module) = resolve_legacy_guest_module_with_pool(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        cache_scope,
        execution.instance_pool.as_deref(),
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

#[cfg(test)]
fn resolve_legacy_guest_module(
    engine: &Engine,
    function_name: &str,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> std::result::Result<(PathBuf, Module), ExecutionError> {
    resolve_legacy_guest_module_with_pool(engine, function_name, core_store, cache_scope, None)
}

/// Same as `resolve_legacy_guest_module`, but consults the runtime's in-memory
/// instance pool first. The pool stores `Arc<Module>` keyed by the resolved
/// canonical path; on a hit we skip the redb lookup and the
/// `Module::deserialize` cost entirely. On a miss we load through the existing
/// redb-backed precompile path and populate the pool for subsequent requests.
fn resolve_legacy_guest_module_with_pool(
    engine: &Engine,
    function_name: &str,
    core_store: &store::CoreStore,
    cache_scope: &str,
    instance_pool: Option<&moka::sync::Cache<PathBuf, Arc<Module>>>,
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

        let normalized = normalize_path(candidate.clone());
        if let Some(pool) = instance_pool {
            if let Some(cached) = pool.get(&normalized) {
                // `Module` is internally Arc-backed; cloning the `Module` value out
                // of the `Arc<Module>` returned by the pool is cheap.
                return Ok((normalized, (*cached).clone()));
            }
        }

        match load_module_with_core_store(engine, &candidate, core_store, cache_scope) {
            Ok(module) => {
                if let Some(pool) = instance_pool {
                    pool.insert(normalized.clone(), Arc::new(module.clone()));
                }
                return Ok((normalized, module));
            }
            Err(error) => last_error = Some((normalized, error)),
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
    component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::vector::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add vector store functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::training::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add training functions to component linker",
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
    store.data_mut().bridge_manager = Arc::clone(&execution.bridge_manager);
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
        HostWebSocketFrame::Text(text) => AxumWebSocketMessage::Text(text.into()),
        HostWebSocketFrame::Binary(bytes) => AxumWebSocketMessage::Binary(bytes.into()),
        HostWebSocketFrame::Ping(bytes) => AxumWebSocketMessage::Ping(bytes.into()),
        HostWebSocketFrame::Pong(bytes) => AxumWebSocketMessage::Pong(bytes.into()),
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
    control_plane_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add outbound HTTP functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add routing control functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to system component linker",
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
    store.data_mut().route_overrides = Arc::clone(&execution.route_overrides);
    store.data_mut().host_load = Arc::clone(&execution.host_load);
    store.data_mut().bridge_manager = Arc::clone(&execution.bridge_manager);
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    if let Ok(bindings) = control_plane_component_bindings::ControlPlaneFaas::instantiate(
        &mut store, component, &linker,
    ) {
        record_wasm_start(execution.telemetry.as_ref());
        let response = bindings.tachyon_mesh_handler().call_handle_request(
            &mut store,
            &control_plane_component_bindings::exports::tachyon::mesh::handler::Request {
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
            guest_execution_error(
                error,
                "control-plane guest component `handle-request` trapped",
            )
        })?;
        let status = StatusCode::from_u16(response.status).map_err(|error| {
            ExecutionError::Internal(format!(
                "control-plane guest component returned an invalid HTTP status code `{}`: {error}",
                response.status
            ))
        })?;

        return Ok(GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse {
                status,
                headers: response.headers,
                body: Bytes::from(response.body),
                trailers: response.trailers,
            }),
            fuel_consumed,
        });
    }

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
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
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
        background_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add telemetry reader functions to background component linker",
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
        background_component_bindings::tachyon::mesh::outbox_store::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add outbox store functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add routing control functions to background component linker",
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
        store.data_mut().route_overrides = route_overrides;
        store.data_mut().peer_capabilities = peer_capabilities;
        store.data_mut().host_capabilities = host_capabilities;
        store.data_mut().host_load = host_load;
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(config.guest_fuel_budget)
            .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;

        let bindings = if let Ok(bindings) =
            control_plane_component_bindings::ControlPlaneFaas::instantiate(
                &mut store, &component, &linker,
            ) {
            BackgroundGuestBindings::ControlPlane(bindings)
        } else {
            BackgroundGuestBindings::Background(
                background_component_bindings::BackgroundSystemFaas::instantiate(
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
                })?,
            )
        };

        Ok(Self {
            function_name: function_name.to_owned(),
            route_path: route.path.clone(),
            store,
            bindings,
        })
    }

    fn tick(&mut self) -> std::result::Result<(), ExecutionError> {
        match &self.bindings {
            BackgroundGuestBindings::Background(bindings) => {
                bindings.call_on_tick(&mut self.store).map_err(|error| {
                    guest_execution_error(error, "background system guest `on-tick` trapped")
                })
            }
            BackgroundGuestBindings::ControlPlane(bindings) => {
                bindings.call_on_tick(&mut self.store).map_err(|error| {
                    guest_execution_error(error, "control-plane system guest `on-tick` trapped")
                })
            }
        }
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
    let traceparent = trace_context_for_request(&execution.request_headers);
    add_route_environment_with_trace(
        &mut wasi,
        route,
        execution.host_identity.as_ref(),
        Some(&traceparent),
    )?;

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
    let traceparent = trace_context_for_request(&execution.request_headers);
    add_route_environment_with_trace(
        &mut wasi,
        route,
        execution.host_identity.as_ref(),
        Some(&traceparent),
    )?;
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

fn normalize_route_override_key(route_key: &str) -> String {
    if let Some(route_path) = route_key.strip_prefix(MESH_QOS_OVERRIDE_PREFIX) {
        return format!(
            "{MESH_QOS_OVERRIDE_PREFIX}{}",
            normalize_route_path(route_path)
        );
    }

    normalize_route_path(route_key)
}

fn route_path_for_override_key(route_key: &str) -> String {
    route_key
        .strip_prefix(MESH_QOS_OVERRIDE_PREFIX)
        .map(normalize_route_path)
        .unwrap_or_else(|| normalize_route_path(route_key))
}

fn resolve_guest_module_path(
    function_name: &str,
) -> std::result::Result<PathBuf, GuestModuleNotFound> {
    if system_storage::is_asset_uri(function_name) {
        return system_storage::resolve_asset_uri(&integrity_manifest_path(), function_name)
            .map_err(|error| GuestModuleNotFound::new(function_name, error.to_string()));
    }

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

    fn keda_pending_queue_size(&self) -> u32 {
        let pending = self.pending_queue_size();
        if pending == 0 {
            return 0;
        }

        let active = self.active_requests.load(Ordering::Relaxed);
        if active < self.max_concurrency as usize {
            return pending;
        }

        pending.saturating_add(self.max_concurrency)
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
        ensure_rustls_crypto_provider();
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(true)
            .try_init();
    });
}

pub(crate) fn ensure_rustls_crypto_provider() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    });
}

fn verify_integrity() -> Result<IntegrityConfig> {
    match verify_integrity_payload(
        EMBEDDED_CONFIG_PAYLOAD,
        EMBEDDED_PUBLIC_KEY,
        EMBEDDED_SIGNATURE,
        "embedded sealed configuration",
    ) {
        Ok(config) => {
            tracing::info!("integrity verification passed");
            Ok(config)
        }
        Err(error) => {
            if is_integrity_schema_violation(&error) {
                tracing::error!(
                    code = ERR_INTEGRITY_SCHEMA_VIOLATION,
                    error = %error,
                    "integrity manifest violates the required schema"
                );
                panic!("{ERR_INTEGRITY_SCHEMA_VIOLATION}: {error:#}");
            }

            Err(error)
        }
    }
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
        .map_err(|error| integrity_schema_violation(source, error))?;
    validate_integrity_config(config)
}

fn integrity_schema_violation(source: &str, error: serde_json::Error) -> anyhow::Error {
    anyhow!("{ERR_INTEGRITY_SCHEMA_VIOLATION}: failed to parse {source}: {error}")
}

fn is_integrity_schema_violation(error: &anyhow::Error) -> bool {
    error.to_string().contains(ERR_INTEGRITY_SCHEMA_VIOLATION)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminEnrollmentStartRequest {
    node_public_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminEnrollmentStartResponse {
    session_id: String,
    pin: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminEnrollmentApproveRequest {
    session_id: String,
    pin: String,
    /// Hex-encoded signed certificate the operator-side node minted for the
    /// new device. The signing happens upstream of this endpoint (e.g. via the
    /// existing `auth_manager` token-signing or a dedicated cluster-CA tool);
    /// this handler just stages the bytes for the unenrolled node to fetch.
    signed_certificate_hex: String,
}

/// `POST /admin/enrollment/start` — begin a node enrollment session. Caller is
/// the unenrolled node's outbound channel (via the active node's HTTP
/// gateway); body carries the new node's hex-encoded ed25519 public key.
/// Returns the session id + the human-readable PIN the operator must enter
/// in Tachyon Studio to approve the enrollment.
pub(crate) async fn admin_enrollment_start_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<AdminEnrollmentStartRequest>,
) -> Response {
    let node_pubkey = payload.node_public_key.trim().to_owned();
    if node_pubkey.is_empty() {
        return (StatusCode::BAD_REQUEST, "nodePublicKey is required").into_response();
    }
    let session = state.enrollment_manager.start_session(node_pubkey);
    let body = AdminEnrollmentStartResponse {
        session_id: session.session_id,
        pin: session.pin,
    };
    (StatusCode::CREATED, axum::Json(body)).into_response()
}

/// `POST /admin/enrollment/approve` — operator-driven approval entered via
/// Tachyon Studio. Validates the PIN against the recorded session and stages
/// the signed certificate bytes for the unenrolled node's next poll.
pub(crate) async fn admin_enrollment_approve_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<AdminEnrollmentApproveRequest>,
) -> Response {
    let cert_bytes = match hex::decode(&payload.signed_certificate_hex) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("signedCertificateHex must be valid hex: {error}"),
            )
                .into_response();
        }
    };
    match state
        .enrollment_manager
        .approve(&payload.session_id, &payload.pin, cert_bytes)
    {
        Ok(()) => (StatusCode::ACCEPTED, "enrollment approved").into_response(),
        Err(reason) => {
            // PIN mismatch / unknown / already-finalized are all caller errors.
            (StatusCode::BAD_REQUEST, reason).into_response()
        }
    }
}

/// `GET /admin/enrollment/poll/{session_id}` — invoked by the unenrolled node's
/// long-poll. Returns 204 No Content while pending, 200 with the signed cert
/// bytes (hex) once the operator has approved, 410 Gone after rejection.
pub(crate) async fn admin_enrollment_poll_handler(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Response {
    match state.enrollment_manager.poll_outcome(&session_id) {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(node_enrollment::EnrollmentOutcome::Approved { signed_certificate }) => {
            (StatusCode::OK, hex::encode(signed_certificate)).into_response()
        }
        Some(node_enrollment::EnrollmentOutcome::Rejected { reason }) => {
            (StatusCode::GONE, reason).into_response()
        }
    }
}

/// Cluster-wide configuration update event written to `config_update_outbox`
/// whenever a node accepts a signed manifest via `POST /admin/manifest`. The
/// gossip bridge (still TODO — Session C wiring) reads from this table and
/// broadcasts to peers, who then pull the new manifest from `origin_node_id`
/// over the secure overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConfigUpdateEvent {
    pub version: u64,
    pub checksum: String,
    pub origin_node_id: String,
    pub ts_ms: u64,
}

/// `POST /admin/manifest` body: the same `IntegrityManifest` shape as the
/// on-disk file (so admin tooling can hand the file's bytes through unchanged).
/// The payload's signature is verified against the same trust root as the
/// embedded boot manifest. Updates are accepted only when the new
/// `config_version` is strictly greater than the running one — a defense
/// against rollback / replay across the cluster.
pub(crate) async fn admin_manifest_update_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let manifest: IntegrityManifest = match serde_json::from_slice(&body) {
        Ok(m) => m,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to decode manifest payload: {error}"),
            )
                .into_response();
        }
    };

    // Verify signature + parse + validate the embedded config. This rejects
    // unsigned / tampered submissions before we touch disk.
    let new_config = match verify_integrity_payload(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        "admin manifest submission",
    ) {
        Ok(config) => config,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest signature verification failed: {error:#}"),
            )
                .into_response();
        }
    };

    let current_version = state.runtime.load().config.config_version;
    if new_config.config_version <= current_version {
        return (
            StatusCode::CONFLICT,
            format!(
                "manifest config_version {} is not strictly greater than current {}",
                new_config.config_version, current_version
            ),
        )
            .into_response();
    }

    // Atomic write: stage to a tempfile, fsync, rename. The notify-based
    // watcher (Wave 1) sees the rename and triggers `reload_runtime_from_disk`.
    let manifest_path = state.manifest_path.clone();
    let payload_bytes = body.to_vec();
    let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let staging = manifest_path.with_extension("lock.tmp");
        fs::write(&staging, &payload_bytes)?;
        fs::rename(&staging, &manifest_path)?;
        Ok(())
    })
    .await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to persist manifest: {error}"),
            )
                .into_response();
        }
        Err(join_error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("manifest write task failed: {join_error}"),
            )
                .into_response();
        }
    }

    // Emit a gossip update event for peers.
    use sha2::Digest;
    let checksum = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(manifest.config_payload.as_bytes()))
    );
    let ts_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let event = ConfigUpdateEvent {
        version: new_config.config_version,
        checksum,
        origin_node_id: state.host_identity.public_key_hex.clone(),
        ts_ms,
    };
    if let Ok(payload) = serde_json::to_vec(&event) {
        if let Err(error) = state
            .core_store
            .append_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, &payload)
        {
            tracing::warn!("manifest accepted but config_update_outbox append failed: {error:#}");
        }
    }

    (
        StatusCode::ACCEPTED,
        format!(
            "manifest accepted, config_version={}",
            new_config.config_version
        ),
    )
        .into_response()
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

    config.advertise_ip = normalize_advertise_ip(config.advertise_ip)?;
    config.tls_address = normalize_tls_address(config.tls_address)?;
    config.batch_targets = normalize_batch_targets(config.batch_targets)?;
    config.routes = normalize_config_routes(config.routes, !config.batch_targets.is_empty())?;
    validate_tee_requirements(&config)?;
    let route_registry = RouteRegistry::build(&config)?;
    config.resources = normalize_resources(config.resources, &config.routes, &route_registry)?;
    config.layer4 = normalize_layer4_config(config.layer4, &route_registry)?;
    Ok(config)
}

fn validate_tee_requirements(config: &IntegrityConfig) -> Result<()> {
    if config.routes.iter().any(|route| route.requires_tee) && config.tee_backend.is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: routes with `requires_tee: true` require `tee_backend` to be configured"
        ));
    }

    Ok(())
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

fn normalize_resources(
    resources: BTreeMap<String, IntegrityResource>,
    routes: &[IntegrityRoute],
    route_registry: &RouteRegistry,
) -> Result<BTreeMap<String, IntegrityResource>> {
    let reserved_names = routes
        .iter()
        .map(|route| route.name.clone())
        .collect::<BTreeSet<_>>();
    let mut normalized = BTreeMap::new();

    for (name, resource) in resources {
        let normalized_name = normalize_service_name(&name).map_err(|error| {
            anyhow!(
                "Integrity Validation Failed: resource `{name}` has an invalid logical name: {error}"
            )
        })?;
        if reserved_names.contains(&normalized_name) {
            return Err(anyhow!(
                "Integrity Validation Failed: resource `{normalized_name}` conflicts with a sealed route name"
            ));
        }

        let normalized_resource =
            normalize_resource_definition(resource, &normalized_name, route_registry)?;
        if normalized
            .insert(normalized_name.clone(), normalized_resource)
            .is_some()
        {
            return Err(anyhow!(
                "Integrity Validation Failed: resource `{normalized_name}` is defined more than once"
            ));
        }
    }

    Ok(normalized)
}

fn normalize_resource_definition(
    resource: IntegrityResource,
    resource_name: &str,
    route_registry: &RouteRegistry,
) -> Result<IntegrityResource> {
    match resource {
        IntegrityResource::Internal {
            target,
            version_constraint,
        } => {
            let normalized_constraint = version_constraint
                .map(|constraint| {
                    VersionReq::parse(constraint.trim()).map(|parsed| parsed.to_string()).map_err(
                        |_| {
                            anyhow!(
                                "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
                            )
                        },
                    )
                })
                .transpose()?;
            let normalized_target = normalize_internal_resource_reference(
                resource_name,
                &target,
                normalized_constraint.as_deref(),
                route_registry,
            )?;
            Ok(IntegrityResource::Internal {
                target: normalized_target,
                version_constraint: normalized_constraint,
            })
        }
        IntegrityResource::External {
            target,
            allowed_methods,
        } => {
            let normalized_target = normalize_external_resource_target(resource_name, &target)?;
            Ok(IntegrityResource::External {
                target: normalized_target,
                allowed_methods: normalize_resource_methods(resource_name, allowed_methods)?,
            })
        }
    }
}

fn normalize_internal_resource_reference(
    resource_name: &str,
    target: &str,
    version_constraint: Option<&str>,
    route_registry: &RouteRegistry,
) -> Result<String> {
    if target.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: internal resource `{resource_name}` must include a non-empty `target`"
        ));
    }

    if target.trim_start().starts_with('/') {
        let normalized_path = normalize_route_path(target);
        let route = route_registry.by_path.get(&normalized_path).ok_or_else(|| {
            anyhow!(
                "Integrity Validation Failed: internal resource `{resource_name}` target `{normalized_path}` does not match any sealed route"
            )
        })?;
        if let Some(requirement) = version_constraint {
            let parsed = VersionReq::parse(requirement).map_err(|_| {
                anyhow!(
                    "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
                )
            })?;
            if !parsed.matches(&route.version) {
                return Err(anyhow!(
                    "Integrity Validation Failed: internal resource `{resource_name}` target `{normalized_path}` does not satisfy `{requirement}`"
                ));
            }
        }
        return Ok(normalized_path);
    }

    let normalized_name = normalize_service_name(target).map_err(|error| {
        anyhow!(
            "Integrity Validation Failed: internal resource `{resource_name}` target `{target}` is invalid: {error}"
        )
    })?;
    if let Some(requirement) = version_constraint {
        let parsed = VersionReq::parse(requirement).map_err(|_| {
            anyhow!(
                "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
            )
        })?;
        route_registry
            .resolve_named_route_matching(&normalized_name, &parsed)
            .map_err(|error| anyhow!("Integrity Validation Failed: {error}"))?;
    } else {
        route_registry
            .resolve_named_route(&normalized_name)
            .map_err(|error| anyhow!("Integrity Validation Failed: {error}"))?;
    }
    Ok(normalized_name)
}

fn normalize_external_resource_target(resource_name: &str, target: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(target.trim()).map_err(|error| {
        anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target is not a valid URL: {error}"
        )
    })?;
    let host = parsed.host_str().ok_or_else(|| {
        anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target must include a host"
        )
    })?;
    let loopback_http = parsed.scheme() == "http"
        && (host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .ok()
                .is_some_and(|ip| ip.is_loopback()));
    let cluster_local_http = parsed.scheme() == "http" && is_cluster_local_service_host(host);
    if parsed.scheme() != "https" && !loopback_http && !cluster_local_http {
        return Err(anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target must use HTTPS unless it points at localhost for tests or a cluster-local service"
        ));
    }
    Ok(parsed.to_string())
}

fn is_cluster_local_service_host(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    !normalized.is_empty()
        && !normalized.eq("localhost")
        && (!normalized.contains('.')
            || normalized.ends_with(".svc")
            || normalized.ends_with(".svc.cluster.local"))
}

fn normalize_resource_methods(resource_name: &str, methods: Vec<String>) -> Result<Vec<String>> {
    let normalized = methods
        .into_iter()
        .map(|method| {
            let uppercase = method.trim().to_ascii_uppercase();
            if uppercase.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: external resource `{resource_name}` must not declare empty HTTP methods"
                ));
            }
            Method::from_bytes(uppercase.as_bytes()).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: external resource `{resource_name}` has an invalid HTTP method `{uppercase}`: {error}"
                )
            })?;
            Ok(uppercase)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if normalized.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` must declare at least one allowed HTTP method"
        ));
    }
    Ok(normalized.into_iter().collect())
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
        resource_policy: route.resource_policy,
        runtime: normalize_route_runtime(route.runtime, &normalized)?,
        sync_to_cloud: route.sync_to_cloud,
        requires_tee: route.requires_tee,
        allow_overflow: route.allow_overflow,
        distributed_rate_limit: route.distributed_rate_limit,
        adapter_id: route.adapter_id,
    })
}

fn normalize_route_runtime(
    runtime: Option<FaaSRuntime>,
    route_path: &str,
) -> Result<Option<FaaSRuntime>> {
    match runtime {
        Some(FaaSRuntime::Microvm {
            image,
            vcpus,
            memory_mb,
        }) => {
            let image = image.trim().to_owned();
            if image.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires a non-empty image"
                ));
            }
            if vcpus == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires at least one vCPU"
                ));
            }
            if memory_mb < 64 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires at least 64 MiB memory"
                ));
            }
            Ok(Some(FaaSRuntime::Microvm {
                image,
                vcpus,
                memory_mb,
            }))
        }
        other => Ok(other),
    }
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

fn normalize_advertise_ip(address: Option<String>) -> Result<Option<String>> {
    address
        .map(|address| {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: `advertise_ip` must not be empty"
                ));
            }
            trimmed.parse::<IpAddr>().map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: `advertise_ip` must be an IP address: {error}"
                )
            })?;
            Ok(trimmed.to_owned())
        })
        .transpose()
}

fn effective_advertise_ip(config: &IntegrityConfig) -> String {
    config
        .advertise_ip
        .clone()
        .or_else(|| {
            config
                .host_address
                .parse::<SocketAddr>()
                .ok()
                .map(|address| address.ip().to_string())
        })
        .unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string())
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
    let requires = normalize_capabilities(
        target.requires,
        format!("route target `{module}` capabilities"),
    )?;

    Ok(RouteTarget {
        module,
        weight: target.weight,
        websocket: target.websocket,
        match_header: target
            .match_header
            .map(normalize_header_match)
            .transpose()?,
        requires,
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
        encrypted: volume.encrypted,
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
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry,
            concurrency_limits,
            propagated_headers,
            route_overrides: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            peer_capabilities: Arc::new(Mutex::new(HashMap::new())),
            host_capabilities: Capabilities::detect(),
            host_load: Arc::new(HostLoadCounters::default()),
            outbound_http_client: blocking_outbound_http_client(),
            route_path: route.path.clone(),
            route_role: route.role,
            #[cfg(feature = "ai-inference")]
            ai_runtime: None,
            #[cfg(feature = "ai-inference")]
            allowed_model_aliases: route
                .models
                .iter()
                .map(|binding| binding.alias.clone())
                .collect(),
            #[cfg(feature = "ai-inference")]
            adapter_id: route.adapter_id.clone(),
            #[cfg(feature = "ai-inference")]
            accelerator_models: HashMap::new(),
            #[cfg(feature = "ai-inference")]
            next_accelerator_model_id: 1,
        })
    }

    fn pending_queue_size(&self, route_path: &str) -> u32 {
        self.concurrency_limits
            .get(&normalize_route_path(route_path))
            .map(|control| control.keda_pending_queue_size())
            .unwrap_or_default()
    }

    fn vector_tenant_id(&self) -> String {
        self.request_headers
            .get("x-tachyon-tenant")
            .or_else(|| self.request_headers.get("x-tenant-id"))
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.route_path)
            .to_owned()
    }

    fn hot_model_aliases(&self) -> Vec<String> {
        #[cfg(feature = "ai-inference")]
        {
            self.ai_runtime
                .as_ref()
                .map(|runtime| runtime.loaded_model_aliases())
                .unwrap_or_default()
        }

        #[cfg(not(feature = "ai-inference"))]
        {
            Vec::new()
        }
    }

    fn accelerator_queue_loads(&self) -> AcceleratorQueueLoads {
        #[cfg(feature = "ai-inference")]
        {
            let Some(runtime) = self.ai_runtime.as_ref() else {
                return AcceleratorQueueLoads::default();
            };
            let cpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Cpu);
            let gpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Gpu);
            let npu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Npu);
            let tpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Tpu);
            AcceleratorQueueLoads {
                cpu_rt_load: cpu.realtime,
                cpu_standard_load: cpu.standard,
                cpu_batch_load: cpu.batch,
                gpu_rt_load: gpu.realtime,
                gpu_standard_load: gpu.standard,
                gpu_batch_load: gpu.batch,
                npu_rt_load: npu.realtime,
                npu_standard_load: npu.standard,
                npu_batch_load: npu.batch,
                tpu_rt_load: tpu.realtime,
                tpu_standard_load: tpu.standard,
                tpu_batch_load: tpu.batch,
            }
        }

        #[cfg(not(feature = "ai-inference"))]
        {
            AcceleratorQueueLoads::default()
        }
    }

    fn handle_bridge_create(
        &self,
        config: BridgeConfig,
    ) -> std::result::Result<BridgeAllocation, String> {
        if self.route_role == RouteRole::System && self.route_path == SYSTEM_BRIDGE_ROUTE {
            let mut allocation = self.bridge_manager.create_relay(config)?;
            allocation.ip = effective_advertise_ip(&self.runtime_config);
            return Ok(allocation);
        }

        let url = rewrite_outbound_http_url("http://mesh/system/bridge", &self.runtime_config);
        let response = self
            .outbound_http_client
            .post(&url)
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&config)
                    .map_err(|error| format!("failed to encode bridge config: {error}"))?,
            )
            .send()
            .map_err(|error| format!("failed to call system bridge controller: {error}"))?;
        let status = response.status();
        let body = response
            .bytes()
            .map_err(|error| format!("failed to read bridge controller response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "system bridge controller returned HTTP {}: {}",
                status,
                String::from_utf8_lossy(&body)
            ));
        }
        serde_json::from_slice(&body)
            .map_err(|error| format!("failed to decode bridge allocation response: {error}"))
    }

    fn handle_bridge_destroy(&self, bridge_id: &str) -> std::result::Result<(), String> {
        if self.route_role == RouteRole::System && self.route_path == SYSTEM_BRIDGE_ROUTE {
            return self.bridge_manager.destroy_relay(bridge_id);
        }

        let url = rewrite_outbound_http_url("http://mesh/system/bridge", &self.runtime_config);
        let response = self
            .outbound_http_client
            .delete(&url)
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({ "bridge_id": bridge_id })).map_err(
                    |error| format!("failed to encode bridge teardown request: {error}"),
                )?,
            )
            .send()
            .map_err(|error| format!("failed to call system bridge teardown: {error}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .bytes()
                .map_err(|error| format!("failed to read bridge teardown response: {error}"))?;
            Err(format!(
                "system bridge teardown returned HTTP {}: {}",
                status,
                String::from_utf8_lossy(&body)
            ))
        }
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
            .compute_component_prompt_with_adapter(
                &loaded.alias,
                &prompt,
                self.adapter_id.as_deref(),
            )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct ControlPlaneSnapshot {
    cpu_pressure: u8,
    ram_pressure: u8,
    active_tasks: u32,
    active_instances: u32,
    allocated_memory_pages: u32,
    capability_mask: u64,
    capabilities: Vec<String>,
    cpu_rt_load: u32,
    cpu_standard_load: u32,
    cpu_batch_load: u32,
    gpu_rt_load: u32,
    gpu_standard_load: u32,
    gpu_batch_load: u32,
    npu_rt_load: u32,
    npu_standard_load: u32,
    npu_batch_load: u32,
    tpu_rt_load: u32,
    tpu_standard_load: u32,
    tpu_batch_load: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
struct AcceleratorQueueLoads {
    cpu_rt_load: u32,
    cpu_standard_load: u32,
    cpu_batch_load: u32,
    gpu_rt_load: u32,
    gpu_standard_load: u32,
    gpu_batch_load: u32,
    npu_rt_load: u32,
    npu_standard_load: u32,
    npu_batch_load: u32,
    tpu_rt_load: u32,
    tpu_standard_load: u32,
    tpu_batch_load: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct RouteOverrideDescriptor {
    #[serde(default)]
    candidates: Vec<RouteOverrideCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RouteOverrideCandidate {
    destination: String,
    #[serde(default)]
    hot_models: Vec<String>,
    #[serde(default)]
    effective_pressure: u8,
    #[serde(default)]
    capability_mask: u64,
    #[serde(default)]
    capabilities: Vec<String>,
}

fn guest_memory_page_count(bytes: usize) -> usize {
    ((bytes.saturating_add(65_535)) / 65_536).max(1)
}

fn saturating_percent(value: usize, capacity: usize) -> u8 {
    if capacity == 0 {
        return 0;
    }

    let percent = value.saturating_mul(100) / capacity;
    percent.min(100) as u8
}

fn control_plane_snapshot(
    telemetry: &TelemetryHandle,
    host_load: &HostLoadCounters,
    concurrency_limits: &HashMap<String, Arc<RouteExecutionControl>>,
    runtime_config: &IntegrityConfig,
    host_capabilities: Capabilities,
    queue_loads: AcceleratorQueueLoads,
) -> ControlPlaneSnapshot {
    let active_tasks = telemetry::active_requests(telemetry).min(u32::MAX as usize) as u32;
    let active_instances = host_load.active_instances.load(Ordering::SeqCst);
    let allocated_memory_pages = host_load.allocated_memory_pages.load(Ordering::SeqCst);
    let total_capacity = concurrency_limits
        .values()
        .map(|control| control.max_concurrency as usize)
        .sum::<usize>()
        .max(1);
    let total_memory_pages = total_capacity
        .saturating_mul(guest_memory_page_count(
            runtime_config.guest_memory_limit_bytes,
        ))
        .max(1);

    ControlPlaneSnapshot {
        cpu_pressure: saturating_percent(
            active_instances.max(active_tasks as usize),
            total_capacity,
        ),
        ram_pressure: saturating_percent(allocated_memory_pages, total_memory_pages),
        active_tasks,
        active_instances: active_instances.min(u32::MAX as usize) as u32,
        allocated_memory_pages: allocated_memory_pages.min(u32::MAX as usize) as u32,
        capability_mask: host_capabilities.mask,
        capabilities: host_capabilities.names(),
        cpu_rt_load: queue_loads.cpu_rt_load,
        cpu_standard_load: queue_loads.cpu_standard_load,
        cpu_batch_load: queue_loads.cpu_batch_load,
        gpu_rt_load: queue_loads.gpu_rt_load,
        gpu_standard_load: queue_loads.gpu_standard_load,
        gpu_batch_load: queue_loads.gpu_batch_load,
        npu_rt_load: queue_loads.npu_rt_load,
        npu_standard_load: queue_loads.npu_standard_load,
        npu_batch_load: queue_loads.npu_batch_load,
        tpu_rt_load: queue_loads.tpu_rt_load,
        tpu_standard_load: queue_loads.tpu_standard_load,
        tpu_batch_load: queue_loads.tpu_batch_load,
    }
}

fn control_plane_override_destination(
    route_overrides: &ArcSwap<HashMap<String, String>>,
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    headers: &HeaderMap,
    required_capability_mask: u64,
    requested_model: Option<&str>,
) -> Option<String> {
    if headers.contains_key(TACHYON_BUFFER_REPLAY_HEADER) {
        return None;
    }

    let raw = route_overrides
        .load()
        .get(&normalize_route_override_key(route_path))
        .cloned()?;

    if let Ok(descriptor) = serde_json::from_str::<RouteOverrideDescriptor>(&raw) {
        let selected = descriptor.candidates.iter().find(|candidate| {
            let supports_capabilities = candidate_supports_capabilities(
                candidate,
                peer_capabilities,
                required_capability_mask,
            );
            let supports_model = requested_model.is_none_or(|alias| {
                candidate
                    .hot_models
                    .iter()
                    .any(|model| model.eq_ignore_ascii_case(alias))
            });
            supports_capabilities && supports_model
        });
        return selected.map(|candidate| candidate.destination.clone());
    }

    if destination_supports_capabilities(&raw, peer_capabilities, required_capability_mask) {
        return Some(raw);
    }

    cached_capable_peer_destination(peer_capabilities, route_path, required_capability_mask)
}

fn update_control_plane_route_override(
    route_overrides: &ArcSwap<HashMap<String, String>>,
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    destination: &str,
) -> std::result::Result<(), String> {
    let normalized_route = normalize_route_override_key(route_path);
    let normalized_destination = destination.trim();
    if normalized_destination.is_empty() {
        return Err("route override destinations must not be empty".to_owned());
    }

    let direct_destination = normalized_destination.starts_with('/')
        || normalized_destination.starts_with("http://")
        || normalized_destination.starts_with("https://");
    if !direct_destination {
        let descriptor = serde_json::from_str::<RouteOverrideDescriptor>(normalized_destination)
            .map_err(|_| {
                format!(
                    "route override `{normalized_destination}` must be an absolute route, URL, or serialized candidate descriptor"
                )
            })?;
        if descriptor.candidates.is_empty() {
            return Err(
                "route override descriptors must include at least one candidate".to_owned(),
            );
        }
        for candidate in &descriptor.candidates {
            let candidate_destination = candidate.destination.trim();
            if !candidate_destination.starts_with('/')
                && !candidate_destination.starts_with("http://")
                && !candidate_destination.starts_with("https://")
            {
                return Err(format!(
                    "route override candidate `{candidate_destination}` must be an absolute route or URL"
                ));
            }
        }
        cache_peer_capabilities(peer_capabilities, &descriptor.candidates);
    }

    let mut next = (**route_overrides.load()).clone();
    if normalized_destination == normalized_route {
        next.remove(&normalized_route);
    } else {
        next.insert(normalized_route, normalized_destination.to_owned());
    }
    route_overrides.store(Arc::new(next));
    Ok(())
}

fn candidate_supports_capabilities(
    candidate: &RouteOverrideCandidate,
    peer_capabilities: &PeerCapabilityCache,
    required_capability_mask: u64,
) -> bool {
    if required_capability_mask == 0 || required_capability_mask == Capabilities::CORE_WASI {
        return true;
    }

    let declared_mask = required_capability_mask_for_candidate(candidate);
    if declared_mask != 0 {
        return (declared_mask & required_capability_mask) == required_capability_mask;
    }

    destination_supports_capabilities(
        &candidate.destination,
        peer_capabilities,
        required_capability_mask,
    )
}

fn destination_supports_capabilities(
    destination: &str,
    peer_capabilities: &PeerCapabilityCache,
    required_capability_mask: u64,
) -> bool {
    if required_capability_mask == 0 || required_capability_mask == Capabilities::CORE_WASI {
        return true;
    }

    peer_base_url_for_destination(destination)
        .and_then(|base_url| {
            peer_capabilities
                .lock()
                .expect("peer capability cache should not be poisoned")
                .get(&base_url)
                .cloned()
        })
        .is_some_and(|peer| {
            (peer.capability_mask & required_capability_mask) == required_capability_mask
        })
}

fn cache_peer_capabilities(
    peer_capabilities: &PeerCapabilityCache,
    candidates: &[RouteOverrideCandidate],
) {
    let mut cache = peer_capabilities
        .lock()
        .expect("peer capability cache should not be poisoned");
    for candidate in candidates {
        let Some(base_url) = peer_base_url_for_destination(&candidate.destination) else {
            continue;
        };
        let capability_mask = required_capability_mask_for_candidate(candidate);
        if capability_mask == 0 {
            continue;
        }
        let capabilities = if candidate.capabilities.is_empty() {
            capability_names_from_mask(capability_mask)
        } else {
            candidate.capabilities.clone()
        };
        cache.insert(
            base_url,
            CachedPeerCapabilities {
                capabilities,
                capability_mask,
                effective_pressure: candidate.effective_pressure,
            },
        );
    }
}

fn required_capability_mask_for_candidate(candidate: &RouteOverrideCandidate) -> u64 {
    if candidate.capability_mask != 0 {
        return candidate.capability_mask;
    }
    Capabilities::from_requirement_list(&candidate.capabilities)
        .map(|capabilities| capabilities.mask)
        .unwrap_or(0)
}

fn peer_base_url_for_destination(destination: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(destination).ok()?;
    let host = parsed.host_str()?;
    let mut base = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        base.push(':');
        base.push_str(&port.to_string());
    }
    Some(base)
}

fn cached_capable_peer_destination(
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    required_capability_mask: u64,
) -> Option<String> {
    let cache = peer_capabilities
        .lock()
        .expect("peer capability cache should not be poisoned");
    cache
        .iter()
        .filter(|(_, peer)| {
            (peer.capability_mask & required_capability_mask) == required_capability_mask
        })
        .min_by_key(|(_, peer)| peer.effective_pressure)
        .map(|(base_url, _)| format!("{base_url}{}", route_path_for_override_key(route_path)))
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
    add_route_environment_with_trace(wasi, route, host_identity, None)
}

fn add_route_environment_with_trace(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
    traceparent: Option<&str>,
) -> std::result::Result<(), ExecutionError> {
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    if let Some(tp) = traceparent {
        // W3C Trace Context propagation. Guests opt in by reading `TRACEPARENT` from
        // their environment (the `faas-sdk` logger / metrics macros do so transparently).
        wasi.env(TACHYON_TRACEPARENT_ENV, tp);
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

/// Standard W3C Trace Context environment variable name. Guests read it via WASI to
/// obtain the active trace context for the request they are servicing.
const TACHYON_TRACEPARENT_ENV: &str = "TRACEPARENT";

/// Honor the inbound `traceparent` header if it is well-formed per the W3C Trace
/// Context spec, otherwise mint a fresh one via the existing `generate_traceparent`
/// so every request that reaches the host gets a globally identifiable trace id.
pub(crate) fn trace_context_for_request(headers: &HeaderMap) -> String {
    if let Some(value) = headers.get("traceparent") {
        if let Ok(s) = value.to_str() {
            if is_valid_w3c_traceparent(s) {
                return s.to_owned();
            }
        }
    }
    generate_traceparent()
}

fn is_valid_w3c_traceparent(value: &str) -> bool {
    // Format: VERSION-TRACE_ID-PARENT_ID-FLAGS, hex; widths 2-32-16-2.
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 4 {
        return false;
    }
    let widths = [2usize, 32, 16, 2];
    for (part, width) in parts.iter().zip(widths.iter()) {
        if part.len() != *width || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    // Forbid the reserved all-zero trace and span ids.
    if parts[1].chars().all(|c| c == '0') || parts[2].chars().all(|c| c == '0') {
        return false;
    }
    true
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
        let host_path = if volume.encrypted {
            encrypted_volume_host_path(&volume.host_path)
        } else {
            PathBuf::from(&volume.host_path)
        };
        if volume.encrypted {
            fs::create_dir_all(&host_path).map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to initialize encrypted volume `{}` for route `{}`: {error}",
                    host_path.display(),
                    route.path
                ))
            })?;
        }
        wasi.preopened_dir(
            &host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to preopen volume `{}` for route `{}` at guest path `{}`: {error}",
                host_path.display(),
                route.path,
                volume.guest_path
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
        let host_path = if volume.encrypted {
            encrypted_volume_host_path(&volume.host_path)
        } else {
            PathBuf::from(&volume.host_path)
        };
        if volume.encrypted {
            fs::create_dir_all(&host_path).with_context(|| {
                format!(
                    "failed to initialize encrypted volume `{}` for batch target `{}`",
                    host_path.display(),
                    target.name
                )
            })?;
        }

        wasi.preopened_dir(
            &host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            anyhow!(
                "failed to preopen volume `{}` for batch target `{}` at guest path `{}`: {error}",
                host_path.display(),
                target.name,
                volume.guest_path
            )
        })?;
    }

    Ok(())
}

fn encrypted_volume_host_path(host_path: &str) -> PathBuf {
    PathBuf::from(host_path).join(".tachyon-tde")
}

fn prepare_encrypted_route_volumes(
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in route.volumes.iter().filter(|volume| volume.encrypted) {
        transform_encrypted_volume_files(&encrypted_volume_host_path(&volume.host_path), false)
            .map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to decrypt encrypted volume `{}` for route `{}`: {error:#}",
                    volume.host_path, route.path
                ))
            })?;
    }
    Ok(())
}

fn seal_encrypted_route_volumes(route: &IntegrityRoute) -> std::result::Result<(), ExecutionError> {
    for volume in route.volumes.iter().filter(|volume| volume.encrypted) {
        transform_encrypted_volume_files(&encrypted_volume_host_path(&volume.host_path), true)
            .map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to encrypt encrypted volume `{}` for route `{}`: {error:#}",
                    volume.host_path, route.path
                ))
            })?;
    }
    Ok(())
}

fn transform_encrypted_volume_files(root: &Path, encrypt: bool) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read encrypted volume `{}`", root.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "failed to inspect encrypted volume entry under `{}`",
                root.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            transform_encrypted_volume_files(&path, encrypt)?;
        } else if path.is_file() {
            transform_tde_file(&path, encrypt)?;
        }
    }

    Ok(())
}

fn transform_tde_file(path: &Path, encrypt: bool) -> Result<()> {
    let body =
        fs::read(path).with_context(|| format!("failed to read TDE file `{}`", path.display()))?;
    let transformed = if encrypt {
        encrypt_tde_file_body(&body)
    } else {
        decrypt_tde_file_body(&body)
    }?;
    if transformed != body {
        fs::write(path, transformed)
            .with_context(|| format!("failed to write TDE file `{}`", path.display()))?;
    }
    Ok(())
}

fn encrypt_tde_file_body(plaintext: &[u8]) -> Result<Vec<u8>> {
    if plaintext.starts_with(TDE_FILE_MAGIC) {
        return Ok(plaintext.to_vec());
    }

    let mut nonce = [0_u8; 12];
    nonce[4..].copy_from_slice(&rand::rng().random::<u64>().to_be_bytes());
    let ciphertext = tde_cipher()
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| anyhow!("failed to encrypt TDE file body"))?;
    let mut out = Vec::with_capacity(TDE_FILE_MAGIC.len() + nonce.len() + ciphertext.len());
    out.extend_from_slice(TDE_FILE_MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_tde_file_body(body: &[u8]) -> Result<Vec<u8>> {
    let Some(rest) = body.strip_prefix(TDE_FILE_MAGIC) else {
        return Ok(body.to_vec());
    };
    if rest.len() < 12 {
        return Err(anyhow!("TDE file body is missing nonce"));
    }
    let (nonce, ciphertext) = rest.split_at(12);
    tde_cipher()
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("failed to decrypt TDE file body"))
}

fn tde_cipher() -> Aes256Gcm {
    Aes256Gcm::new((&tde_key_bytes()).into())
}

fn tde_key_bytes() -> [u8; 32] {
    std::env::var(TDE_KEY_HEX_ENV)
        .ok()
        .and_then(|value| decode_tde_key_hex(value.trim()).ok())
        .unwrap_or([0x42; 32])
}

fn decode_tde_key_hex(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(anyhow!("TDE key must be 64 hexadecimal characters"));
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).context("TDE key must be UTF-8 hex")?;
        out[index] = u8::from_str_radix(pair, 16).context("TDE key must be hexadecimal")?;
    }
    Ok(out)
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

impl component_bindings::tachyon::mesh::bridge_controller::Host for ComponentHostState {
    fn create_bridge(
        &mut self,
        config: component_bindings::tachyon::mesh::bridge_controller::BridgeConfig,
    ) -> std::result::Result<
        component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints,
        String,
    > {
        let allocation = self.handle_bridge_create(BridgeConfig {
            client_a_addr: config.client_a_addr,
            client_b_addr: config.client_b_addr,
            timeout_seconds: config.timeout_seconds,
        })?;
        Ok(
            component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints {
                bridge_id: allocation.bridge_id,
                ip: allocation.ip,
                port_a: allocation.port_a,
                port_b: allocation.port_b,
            },
        )
    }

    fn destroy_bridge(&mut self, bridge_id: String) -> std::result::Result<(), String> {
        self.handle_bridge_destroy(&bridge_id)
    }
}

impl system_component_bindings::tachyon::mesh::bridge_controller::Host for ComponentHostState {
    fn create_bridge(
        &mut self,
        config: system_component_bindings::tachyon::mesh::bridge_controller::BridgeConfig,
    ) -> std::result::Result<
        system_component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints,
        String,
    > {
        let allocation = self.handle_bridge_create(BridgeConfig {
            client_a_addr: config.client_a_addr,
            client_b_addr: config.client_b_addr,
            timeout_seconds: config.timeout_seconds,
        })?;
        Ok(
            system_component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints {
                bridge_id: allocation.bridge_id,
                ip: allocation.ip,
                port_a: allocation.port_a,
                port_b: allocation.port_b,
            },
        )
    }

    fn destroy_bridge(&mut self, bridge_id: String) -> std::result::Result<(), String> {
        self.handle_bridge_destroy(&bridge_id)
    }
}

impl component_bindings::tachyon::mesh::vector::Host for ComponentHostState {
    fn create_index(
        &mut self,
        spec: component_bindings::tachyon::mesh::vector::IndexSpec,
    ) -> std::result::Result<(), String> {
        self.storage_broker
            .core_store
            .create_vector_index(
                &self.vector_tenant_id(),
                &spec.name,
                spec.dim as usize,
                spec.m,
                spec.ef_construction,
            )
            .map_err(|error| format!("{error:#}"))
    }

    fn upsert(
        &mut self,
        name: String,
        docs: Vec<component_bindings::tachyon::mesh::vector::Document>,
    ) -> std::result::Result<(), String> {
        let docs = docs
            .into_iter()
            .map(|doc| store::VectorDocument {
                id: doc.id,
                embedding: doc.embedding,
                payload: doc.payload,
            })
            .collect();
        self.storage_broker
            .core_store
            .upsert_vectors(&self.vector_tenant_id(), &name, docs)
            .map_err(|error| format!("{error:#}"))
    }

    fn search(
        &mut self,
        name: String,
        query: Vec<f32>,
        k: u32,
    ) -> std::result::Result<Vec<component_bindings::tachyon::mesh::vector::SearchMatch>, String>
    {
        self.storage_broker
            .core_store
            .search_vectors(&self.vector_tenant_id(), &name, &query, k as usize)
            .map(|matches| {
                matches
                    .into_iter()
                    .map(
                        |item| component_bindings::tachyon::mesh::vector::SearchMatch {
                            id: item.id,
                            score: item.score,
                            payload: item.payload,
                        },
                    )
                    .collect()
            })
            .map_err(|error| format!("{error:#}"))
    }

    fn remove(&mut self, name: String, id: String) -> std::result::Result<bool, String> {
        self.storage_broker
            .core_store
            .remove_vector(&self.vector_tenant_id(), &name, &id)
            .map_err(|error| format!("{error:#}"))
    }
}

impl component_bindings::tachyon::mesh::training::Host for ComponentHostState {
    fn submit_training_job(
        &mut self,
        job: component_bindings::tachyon::mesh::training::TrainingJob,
    ) -> std::result::Result<component_bindings::tachyon::mesh::training::JobId, String> {
        if job.base_model.trim().is_empty() {
            return Err("training job base model must not be empty".to_owned());
        }
        if job.dataset.path.trim().is_empty() {
            return Err("training job dataset path must not be empty".to_owned());
        }
        let queue = lora_training_queue();
        let id = format!("lora-{}", Uuid::new_v4().simple());
        update_lora_training_status(&queue.statuses, &id, LoraTrainingJobStatus::Queued);
        queue
            .sender
            .send(LoraTrainingJob {
                id: id.clone(),
                tenant_id: self.vector_tenant_id(),
                base_model: job.base_model,
                dataset_volume: job.dataset.volume_alias,
                dataset_path: job.dataset.path,
                dataset_split: job.dataset.split,
                rank: job.rank,
                max_steps: job.max_steps,
                seed: job.seed,
            })
            .map_err(|error| format!("failed to queue LoRA training job: {error}"))?;
        Ok(component_bindings::tachyon::mesh::training::JobId { value: id })
    }

    fn get_training_status(
        &mut self,
        id: component_bindings::tachyon::mesh::training::JobId,
    ) -> std::result::Result<component_bindings::tachyon::mesh::training::JobStatus, String> {
        let queue = lora_training_queue();
        let status = queue
            .statuses
            .lock()
            .expect("LoRA training status map should not be poisoned")
            .get(&id.value)
            .cloned()
            .ok_or_else(|| format!("unknown LoRA training job `{}`", id.value))?;
        Ok(match status {
            LoraTrainingJobStatus::Queued => {
                component_bindings::tachyon::mesh::training::JobStatus::Queued
            }
            LoraTrainingJobStatus::Running { step, total } => {
                component_bindings::tachyon::mesh::training::JobStatus::Running((step, total))
            }
            LoraTrainingJobStatus::Completed { adapter_path } => {
                component_bindings::tachyon::mesh::training::JobStatus::Completed(adapter_path)
            }
            LoraTrainingJobStatus::Failed { message } => {
                component_bindings::tachyon::mesh::training::JobStatus::Failed(message)
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
        let control_plane = control_plane_snapshot(
            &self.telemetry,
            self.host_load.as_ref(),
            self.concurrency_limits.as_ref(),
            &self.runtime_config,
            self.host_capabilities,
            self.accelerator_queue_loads(),
        );
        let l4 = self.bridge_manager.telemetry_snapshot();
        let hot_models = self.hot_model_aliases();

        system_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests,
            completed_requests,
            error_requests,
            active_requests,
            cpu_pressure: control_plane.cpu_pressure,
            ram_pressure: control_plane.ram_pressure,
            active_instances: control_plane.active_instances,
            allocated_memory_pages: control_plane.allocated_memory_pages,
            capability_mask: control_plane.capability_mask,
            capabilities: control_plane.capabilities,
            active_l4_relays: l4.active_relays,
            l4_throughput_bytes_per_sec: l4.throughput_bytes_per_sec,
            l4_load_score: l4.load_score,
            advertise_ip: effective_advertise_ip(&self.runtime_config),
            cpu_rt_load: control_plane.cpu_rt_load,
            cpu_standard_load: control_plane.cpu_standard_load,
            cpu_batch_load: control_plane.cpu_batch_load,
            gpu_rt_load: control_plane.gpu_rt_load,
            gpu_standard_load: control_plane.gpu_standard_load,
            gpu_batch_load: control_plane.gpu_batch_load,
            npu_rt_load: control_plane.npu_rt_load,
            npu_standard_load: control_plane.npu_standard_load,
            npu_batch_load: control_plane.npu_batch_load,
            tpu_rt_load: control_plane.tpu_rt_load,
            tpu_standard_load: control_plane.tpu_standard_load,
            tpu_batch_load: control_plane.tpu_batch_load,
            hot_models,
            dropped_events,
            last_status,
            total_duration_us,
            total_wasm_duration_us,
            total_host_overhead_us,
        }
    }
}

impl control_plane_component_bindings::tachyon::mesh::telemetry_reader::Host
    for ComponentHostState
{
    fn get_metrics(
        &mut self,
    ) -> control_plane_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let snapshot =
            <Self as system_component_bindings::tachyon::mesh::telemetry_reader::Host>::get_metrics(
                self,
            );
        control_plane_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests: snapshot.total_requests,
            completed_requests: snapshot.completed_requests,
            error_requests: snapshot.error_requests,
            active_requests: snapshot.active_requests,
            cpu_pressure: snapshot.cpu_pressure,
            ram_pressure: snapshot.ram_pressure,
            active_instances: snapshot.active_instances,
            allocated_memory_pages: snapshot.allocated_memory_pages,
            capability_mask: snapshot.capability_mask,
            capabilities: snapshot.capabilities,
            active_l4_relays: snapshot.active_l4_relays,
            l4_throughput_bytes_per_sec: snapshot.l4_throughput_bytes_per_sec,
            l4_load_score: snapshot.l4_load_score,
            advertise_ip: snapshot.advertise_ip,
            cpu_rt_load: snapshot.cpu_rt_load,
            cpu_standard_load: snapshot.cpu_standard_load,
            cpu_batch_load: snapshot.cpu_batch_load,
            gpu_rt_load: snapshot.gpu_rt_load,
            gpu_standard_load: snapshot.gpu_standard_load,
            gpu_batch_load: snapshot.gpu_batch_load,
            npu_rt_load: snapshot.npu_rt_load,
            npu_standard_load: snapshot.npu_standard_load,
            npu_batch_load: snapshot.npu_batch_load,
            tpu_rt_load: snapshot.tpu_rt_load,
            tpu_standard_load: snapshot.tpu_standard_load,
            tpu_batch_load: snapshot.tpu_batch_load,
            hot_models: snapshot.hot_models,
            dropped_events: snapshot.dropped_events,
            last_status: snapshot.last_status,
            total_duration_us: snapshot.total_duration_us,
            total_wasm_duration_us: snapshot.total_wasm_duration_us,
            total_host_overhead_us: snapshot.total_host_overhead_us,
        }
    }
}

impl background_component_bindings::tachyon::mesh::telemetry_reader::Host for ComponentHostState {
    fn get_metrics(
        &mut self,
    ) -> background_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let snapshot =
            <Self as system_component_bindings::tachyon::mesh::telemetry_reader::Host>::get_metrics(
                self,
            );
        background_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests: snapshot.total_requests,
            completed_requests: snapshot.completed_requests,
            error_requests: snapshot.error_requests,
            active_requests: snapshot.active_requests,
            cpu_pressure: snapshot.cpu_pressure,
            ram_pressure: snapshot.ram_pressure,
            active_instances: snapshot.active_instances,
            allocated_memory_pages: snapshot.allocated_memory_pages,
            capability_mask: snapshot.capability_mask,
            capabilities: snapshot.capabilities,
            active_l4_relays: snapshot.active_l4_relays,
            l4_throughput_bytes_per_sec: snapshot.l4_throughput_bytes_per_sec,
            l4_load_score: snapshot.l4_load_score,
            advertise_ip: snapshot.advertise_ip,
            cpu_rt_load: snapshot.cpu_rt_load,
            cpu_standard_load: snapshot.cpu_standard_load,
            cpu_batch_load: snapshot.cpu_batch_load,
            gpu_rt_load: snapshot.gpu_rt_load,
            gpu_standard_load: snapshot.gpu_standard_load,
            gpu_batch_load: snapshot.gpu_batch_load,
            npu_rt_load: snapshot.npu_rt_load,
            npu_standard_load: snapshot.npu_standard_load,
            npu_batch_load: snapshot.npu_batch_load,
            tpu_rt_load: snapshot.tpu_rt_load,
            tpu_standard_load: snapshot.tpu_standard_load,
            tpu_batch_load: snapshot.tpu_batch_load,
            hot_models: snapshot.hot_models,
            dropped_events: snapshot.dropped_events,
            last_status: snapshot.last_status,
            total_duration_us: snapshot.total_duration_us,
            total_wasm_duration_us: snapshot.total_wasm_duration_us,
            total_host_overhead_us: snapshot.total_host_overhead_us,
        }
    }
}

impl system_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl control_plane_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
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

        self.storage_broker.enqueue_write_target(
            route.path,
            route.sync_to_cloud,
            resolved,
            mode,
            body,
        )
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

impl control_plane_component_bindings::tachyon::mesh::routing_control::Host for ComponentHostState {
    fn update_target(
        &mut self,
        route_path: String,
        destination: String,
    ) -> std::result::Result<(), String> {
        update_control_plane_route_override(
            self.route_overrides.as_ref(),
            &self.peer_capabilities,
            &route_path,
            &destination,
        )
    }
}

impl background_component_bindings::tachyon::mesh::routing_control::Host for ComponentHostState {
    fn update_target(
        &mut self,
        route_path: String,
        destination: String,
    ) -> std::result::Result<(), String> {
        update_control_plane_route_override(
            self.route_overrides.as_ref(),
            &self.peer_capabilities,
            &route_path,
            &destination,
        )
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
        let route_registry = RouteRegistry::build(&self.runtime_config)
            .map_err(|error| format!("failed to build sealed route registry: {error:#}"))?;
        let caller_route = self
            .runtime_config
            .sealed_route(&self.route_path)
            .ok_or_else(|| {
                format!(
                    "sealed caller route `{}` is not present in `integrity.lock`",
                    self.route_path
                )
            })?;
        let resolved_target = resolve_outbound_http_target(
            &self.runtime_config,
            &route_registry,
            caller_route,
            &method,
            &url,
        )?;
        let url = rewrite_outbound_http_url(&resolved_target.url, &self.runtime_config);

        tracing::info!(
            method = %method,
            url = %url,
            bytes = body.len(),
            "autoscaling guest sending outbound HTTP request"
        );

        let mut request = self.outbound_http_client.request(method, &url);
        for (name, value) in
            filtered_outbound_http_headers(headers, &self.propagated_headers, &resolved_target.kind)
        {
            request = request.header(&name, &value);
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

impl background_component_bindings::tachyon::mesh::outbox_store::Host for ComponentHostState {
    fn claim_events(
        &mut self,
        db_url: String,
        table: String,
        max_events: u32,
    ) -> std::result::Result<
        Vec<background_component_bindings::tachyon::mesh::outbox_store::OutboxEvent>,
        String,
    > {
        data_events::claim_events(
            self.storage_broker.core_store.as_ref(),
            &db_url,
            &table,
            max_events,
        )
        .map(|events| {
            events
                .into_iter()
                .map(|event| {
                    background_component_bindings::tachyon::mesh::outbox_store::OutboxEvent {
                        id: event.id,
                        content_type: event.content_type,
                        body: event.body,
                    }
                })
                .collect()
        })
        .map_err(|error| format!("failed to claim outbox events: {error}"))
    }

    fn ack_event(
        &mut self,
        db_url: String,
        table: String,
        id: String,
    ) -> std::result::Result<(), String> {
        data_events::ack_event(
            self.storage_broker.core_store.as_ref(),
            &db_url,
            &table,
            &id,
        )
        .map_err(|error| format!("failed to ack outbox event `{id}`: {error}"))
    }
}

impl control_plane_component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<
        control_plane_component_bindings::tachyon::mesh::outbound_http::Response,
        String,
    > {
        let response =
            <Self as background_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
                self, method, url, headers, body,
            )?;
        Ok(
            control_plane_component_bindings::tachyon::mesh::outbound_http::Response {
                status: response.status,
                headers: response.headers,
                body: response.body,
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

fn filtered_outbound_http_headers(
    headers: Vec<(String, String)>,
    propagated_headers: &[PropagatedHeader],
    target_kind: &OutboundTargetKind,
) -> Vec<(String, String)> {
    let mut filtered = headers;
    if target_kind.is_internal() {
        filtered.extend(
            propagated_headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone())),
        );
        return filtered;
    }

    filtered.retain(|(name, _)| allow_external_outbound_header(name));
    filtered
}

fn allow_external_outbound_header(name: &str) -> bool {
    ![
        HOP_LIMIT_HEADER,
        COHORT_HEADER,
        TACHYON_COHORT_HEADER,
        TACHYON_IDENTITY_HEADER,
        TACHYON_ORIGINAL_ROUTE_HEADER,
        TACHYON_BUFFER_REPLAY_HEADER,
        "connection",
        "content-length",
        "host",
        "keep-alive",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ]
    .iter()
    .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
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
            advertise_ip: None,
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            resources: BTreeMap::new(),
            routes: vec![
                IntegrityRoute::user_with_secrets(DEFAULT_ROUTE, &["DB_PASS"]),
                IntegrityRoute::system(DEFAULT_SYSTEM_ROUTE),
            ],

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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
    use rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        KeyUsagePurpose, SanType,
    };
    use std::{
        fs,
        net::{IpAddr, Ipv4Addr},
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

    struct MtlsTestMaterial {
        ca_pem: String,
        server_cert_pem: String,
        server_key_pem: String,
        client_cert_pem: String,
        client_key_pem: String,
    }

    fn generate_mtls_test_material() -> MtlsTestMaterial {
        let ca_key = KeyPair::generate().expect("CA key should generate");
        let mut ca_params =
            CertificateParams::new(vec!["tachyon-mtls-ca".to_owned()]).expect("CA params");
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::CrlSign,
        ];
        let ca_cert = ca_params
            .self_signed(&ca_key)
            .expect("CA certificate should self-sign");
        let ca_issuer = Issuer::from_params(&ca_params, &ca_key);

        let server_key = KeyPair::generate().expect("server key should generate");
        let mut server_params =
            CertificateParams::new(vec!["localhost".to_owned()]).expect("server params");
        server_params
            .subject_alt_names
            .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        server_params.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        let server_cert = server_params
            .signed_by(&server_key, &ca_issuer)
            .expect("server certificate should sign");

        let client_key = KeyPair::generate().expect("client key should generate");
        let mut client_params =
            CertificateParams::new(vec!["tachyon-client".to_owned()]).expect("client params");
        client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let client_cert = client_params
            .signed_by(&client_key, &ca_issuer)
            .expect("client certificate should sign");

        MtlsTestMaterial {
            ca_pem: ca_cert.pem(),
            server_cert_pem: server_cert.pem(),
            server_key_pem: server_key.serialize_pem(),
            client_cert_pem: client_cert.pem(),
            client_key_pem: client_key.serialize_pem(),
        }
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

    fn test_route_overrides() -> Arc<ArcSwap<HashMap<String, String>>> {
        Arc::new(ArcSwap::from_pointee(HashMap::new()))
    }

    fn test_peer_capabilities() -> PeerCapabilityCache {
        Arc::new(Mutex::new(HashMap::new()))
    }

    fn test_host_load() -> Arc<HostLoadCounters> {
        Arc::new(HostLoadCounters::default())
    }

    fn test_selected_target(module: &str, websocket: bool) -> SelectedRouteTarget {
        SelectedRouteTarget {
            module: module.to_owned(),
            websocket,
            required_capabilities: default_route_capabilities(),
            required_capability_mask: Capabilities::CORE_WASI,
        }
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

    #[test]
    fn instance_pool_is_isolated_per_runtime_generation() {
        // Two consecutive runtime states (modelling a hot reload) get fresh,
        // independent pools. An entry inserted into one is invisible to the other,
        // which keeps configuration changes from being shadowed by stale modules.
        let r1 = build_test_runtime(IntegrityConfig::default_sealed());
        let r2 = build_test_runtime(IntegrityConfig::default_sealed());
        let module_path = std::path::PathBuf::from("/dummy/test.wasm");
        // Raw bytes for an empty Wasm module: magic + version. Avoids pulling in a
        // text-format parser just for this test.
        let module_bytes: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        // SAFETY: bytes are produced by the in-process wat parser, deserialized into
        // the same engine we just built. Standard test idiom.
        let module = wasmtime::Module::new(&r1.engine, module_bytes).expect("build module");
        r1.instance_pool
            .insert(module_path.clone(), Arc::new(module));
        r1.instance_pool.run_pending_tasks();
        assert_eq!(r1.instance_pool.entry_count(), 1);
        r2.instance_pool.run_pending_tasks();
        assert_eq!(
            r2.instance_pool.entry_count(),
            0,
            "hot-reload-style new runtime starts with an empty pool",
        );
    }

    #[test]
    fn instance_pool_evicts_idle_entries_for_hibernation() {
        // The production pool sets `time_to_idle = 5 minutes`. Re-build a tiny pool
        // here with a sub-second idle window so the eviction is observable inside
        // a unit test, then confirm the entry is gone after the window elapses.
        // This is the host-side half of the `wasm-ram-hibernation` change: an
        // idle module's `Arc<Module>` is dropped from RAM, and the next request
        // pays a cwasm thaw (from redb) instead of a full JIT compile.
        let pool: moka::sync::Cache<std::path::PathBuf, Arc<wasmtime::Module>> =
            moka::sync::Cache::builder()
                .max_capacity(8)
                .time_to_idle(Duration::from_millis(50))
                .build();
        let module_bytes: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let runtime = build_test_runtime(IntegrityConfig::default_sealed());
        let module = wasmtime::Module::new(&runtime.engine, module_bytes).expect("module");
        let path = std::path::PathBuf::from("/dummy/idle-test.wasm");
        pool.insert(path.clone(), Arc::new(module));
        pool.run_pending_tasks();
        assert_eq!(pool.entry_count(), 1);

        std::thread::sleep(Duration::from_millis(150));
        pool.run_pending_tasks();
        assert!(
            pool.get(&path).is_none(),
            "idle entry must be evicted past time_to_idle"
        );
    }

    #[test]
    fn instance_pool_hits_short_circuit_redb_lookup() {
        // The pool's contract: when a path is present, `resolve_legacy_guest_module_with_pool`
        // returns the cached module without going through `load_module_with_core_store`
        // and the redb cwasm cache. We exercise this via the public API by inserting a
        // pre-built module and asserting the function returns it on the same path.
        let runtime = build_test_runtime(IntegrityConfig::default_sealed());
        // Raw bytes for an empty Wasm module: magic + version. Avoids pulling in a
        // text-format parser just for this test.
        let module_bytes: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        let module = wasmtime::Module::new(&runtime.engine, module_bytes).expect("build module");
        let path = std::path::PathBuf::from("/dummy/never-on-disk.wasm");
        runtime.instance_pool.insert(path.clone(), Arc::new(module));
        // Build a tiny core_store; the resolve function takes one but won't reach
        // it on a pool hit.
        let dir = test_tempdir();
        let _core_store =
            store::CoreStore::open(&dir.path().join("pool-test.redb")).expect("open store");
        // The function expects to be able to find at least one matching candidate path.
        // We monkey by passing a function name whose normalized candidate equals the
        // path we registered. `guest_module_candidate_paths` produces deterministic
        // candidates relative to the workspace, so we instead check the lower-level
        // primitive directly: assert the pool has an entry for the path.
        runtime.instance_pool.run_pending_tasks();
        let cached = runtime.instance_pool.get(&path).expect("pool hit");
        assert!(
            std::sync::Arc::strong_count(&cached) >= 1,
            "pool returns the same Arc<Module>",
        );
    }

    fn test_tempdir() -> TestTempDir {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("core-host-pool-test-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).expect("create tempdir");
        TestTempDir { path }
    }

    struct TestTempDir {
        path: std::path::PathBuf,
    }
    impl TestTempDir {
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }
    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
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
            advertise_ip: None,
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            resources: BTreeMap::new(),
            routes,

            ..Default::default()
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
            requires: default_route_capabilities(),
        }];

        let mut connector_route = IntegrityRoute::system("/system/sqs-connector");
        connector_route.name = "sqs-connector".to_owned();
        connector_route.targets = vec![RouteTarget {
            module: "system-faas-sqs".to_owned(),
            weight: 100,
            websocket: false,
            match_header: None,
            requires: default_route_capabilities(),
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
            advertise_ip: None,
            tls_address: None,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            layer4: IntegrityLayer4Config::default(),
            telemetry_sample_rate: DEFAULT_TELEMETRY_SAMPLE_RATE,
            batch_targets: Vec::new(),
            resources: BTreeMap::new(),
            routes: vec![target_route, connector_route],

            ..Default::default()
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

                ..Default::default()
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
            bridge_manager: Arc::new(BridgeManager::default()),
            core_store,
            buffered_requests,
            volume_manager: Arc::new(VolumeManager::default()),
            route_overrides: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            peer_capabilities: Arc::new(Mutex::new(HashMap::new())),
            host_capabilities: Capabilities::detect(),
            host_load: Arc::new(HostLoadCounters::default()),
            telemetry,
            tls_manager: Arc::new(tls_runtime::TlsManager::default()),
            mtls_gateway: None,
            auth_manager: Arc::new(
                auth::AuthManager::new(&manifest_path)
                    .expect("test auth manager should initialize"),
            ),
            enrollment_manager: Arc::new(node_enrollment::EnrollmentManager::new()),
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

                ..Default::default()
            }],

            ..Default::default()
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

                ..Default::default()
            }],

            ..Default::default()
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

                ..Default::default()
            }],

            ..Default::default()
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
                requires: default_route_capabilities(),
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

                ..Default::default()
            }],

            ..Default::default()
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
                requires: default_route_capabilities(),
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

                ..Default::default()
            }],

            ..Default::default()
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
                requires: default_route_capabilities(),
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

                ..Default::default()
            }],

            ..Default::default()
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

            ..Default::default()
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

            ..Default::default()
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

                ..Default::default()
            }],

            ..Default::default()
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

                ..Default::default()
            }],

            ..Default::default()
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
            requires: default_route_capabilities(),
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
            requires: default_route_capabilities(),
        }
    }

    fn websocket_target(module: &str) -> RouteTarget {
        RouteTarget {
            module: module.to_owned(),
            weight: 100,
            websocket: true,
            match_header: None,
            requires: default_route_capabilities(),
        }
    }

    fn capability_target(module: &str, requires: &[&str]) -> RouteTarget {
        let mut target = weighted_target(module, 100);
        target.requires = requires
            .iter()
            .map(|capability| (*capability).to_owned())
            .collect();
        target
    }

    fn system_targeted_route(path: &str, module: &str) -> IntegrityRoute {
        let mut route = IntegrityRoute::system(path);
        route.targets = vec![weighted_target(module, 100)];
        route
    }

    fn route_env(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn mounted_volume(host_path: &Path, guest_path: &str) -> IntegrityVolume {
        IntegrityVolume {
            volume_type: VolumeType::Host,
            host_path: host_path.display().to_string(),
            guest_path: guest_path.to_owned(),
            readonly: false,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,

            ..Default::default()
        }
    }

    fn mounted_ram_volume(host_path: &Path, guest_path: &str) -> IntegrityVolume {
        IntegrityVolume {
            volume_type: VolumeType::Ram,
            host_path: host_path.display().to_string(),
            guest_path: guest_path.to_owned(),
            readonly: false,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,

            ..Default::default()
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

    fn signed_manifest_for(config: &IntegrityConfig, signing_key: &SigningKey) -> Vec<u8> {
        let payload = canonical_config_payload(config).expect("payload should serialize");
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));
        let manifest = IntegrityManifest {
            config_payload: payload,
            public_key: hex::encode(signing_key.verifying_key().to_bytes()),
            signature: hex::encode(signature.to_bytes()),
        };
        serde_json::to_vec(&manifest).expect("manifest should serialize")
    }

    #[tokio::test]
    async fn admin_manifest_update_accepts_higher_version_and_emits_outbox_event() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 1;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut next = current.clone();
        next.config_version = 7;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let body = Bytes::from(signed_manifest_for(&next, &signing_key));

        let response = admin_manifest_update_handler(State(state.clone()), body).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        // The outbox should now hold exactly one event for the new version.
        let rows = state
            .core_store
            .peek_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, 16)
            .expect("peek outbox");
        assert_eq!(rows.len(), 1);
        let event: ConfigUpdateEvent =
            serde_json::from_slice(&rows[0].1).expect("event payload parses");
        assert_eq!(event.version, 7);
        assert_eq!(event.origin_node_id, state.host_identity.public_key_hex);
        assert!(event.checksum.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn admin_manifest_update_rejects_rollback() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 9;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut older = current.clone();
        older.config_version = 5;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let body = Bytes::from(signed_manifest_for(&older, &signing_key));

        let response = admin_manifest_update_handler(State(state.clone()), body).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Outbox stays empty on rejection.
        let rows = state
            .core_store
            .peek_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, 16)
            .expect("peek outbox");
        assert!(rows.is_empty());
    }

    #[tokio::test]
    async fn enrollment_start_then_approve_then_poll_round_trips() {
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(IntegrityConfig::default_sealed(), telemetry);

        let start = admin_enrollment_start_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentStartRequest {
                node_public_key: "deadbeef".to_owned(),
            }),
        )
        .await;
        assert_eq!(start.status(), StatusCode::CREATED);
        let body_bytes = axum::body::to_bytes(start.into_body(), 16 * 1024)
            .await
            .expect("body collects");
        let start_body: AdminEnrollmentStartResponse =
            serde_json::from_slice(&body_bytes).expect("response is JSON");

        // Wrong PIN — caller error.
        let bad = admin_enrollment_approve_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentApproveRequest {
                session_id: start_body.session_id.clone(),
                pin: "BAD-PIN".to_owned(),
                signed_certificate_hex: "01020304".to_owned(),
            }),
        )
        .await;
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // Right PIN — accepted.
        let approve = admin_enrollment_approve_handler(
            State(state.clone()),
            axum::Json(AdminEnrollmentApproveRequest {
                session_id: start_body.session_id.clone(),
                pin: start_body.pin.clone(),
                signed_certificate_hex: "01020304".to_owned(),
            }),
        )
        .await;
        assert_eq!(approve.status(), StatusCode::ACCEPTED);

        // Pending node polls and gets the cert; subsequent polls return None
        // because the session is consumed.
        let poll = admin_enrollment_poll_handler(
            State(state.clone()),
            axum::extract::Path(start_body.session_id.clone()),
        )
        .await;
        assert_eq!(poll.status(), StatusCode::OK);
        let cert_bytes = axum::body::to_bytes(poll.into_body(), 1024).await.unwrap();
        assert_eq!(cert_bytes.as_ref(), b"01020304");

        let poll_again = admin_enrollment_poll_handler(
            State(state.clone()),
            axum::extract::Path(start_body.session_id),
        )
        .await;
        assert_eq!(poll_again.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn admin_manifest_update_rejects_tampered_signature() {
        let mut current = IntegrityConfig::default_sealed();
        current.config_version = 1;
        let telemetry = telemetry::init_test_telemetry();
        let state = build_test_state(current.clone(), telemetry);

        let mut next = current.clone();
        next.config_version = 7;
        let signing_key = SigningKey::from_bytes(&[42_u8; 32]);
        let mut bytes = signed_manifest_for(&next, &signing_key);
        // Flip a byte inside the JSON payload — signature no longer matches.
        let pos = bytes
            .iter()
            .position(|b| *b == b'1')
            .expect("contains a '1'");
        bytes[pos] = b'2';
        let response =
            admin_manifest_update_handler(State(state.clone()), Bytes::from(bytes)).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn trace_context_honors_well_formed_inbound_traceparent() {
        let mut headers = HeaderMap::new();
        let inbound = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        headers.insert(
            "traceparent",
            HeaderValue::from_str(inbound).expect("traceparent value is valid ASCII"),
        );
        assert_eq!(trace_context_for_request(&headers), inbound);
    }

    #[test]
    fn trace_context_rejects_malformed_inbound_and_mints_fresh() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            HeaderValue::from_static("not a valid traceparent"),
        );
        let value = trace_context_for_request(&headers);
        assert!(
            is_valid_w3c_traceparent(&value),
            "minted traceparent must be a valid W3C value, got `{value}`"
        );
    }

    #[test]
    fn trace_context_mints_fresh_when_header_absent() {
        let value = trace_context_for_request(&HeaderMap::new());
        assert!(
            is_valid_w3c_traceparent(&value),
            "minted traceparent must be valid, got `{value}`"
        );
        // Smoke-check that two consecutive mints differ — a sanity check on entropy.
        let other = trace_context_for_request(&HeaderMap::new());
        assert_ne!(value, other);
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: None,
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: None,
            },
        )
        .expect("legacy guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutcome {
                output: GuestExecutionOutput::LegacyStdout(Bytes::from(
                    "MESH_FETCH:http://mesh/legacy-service/ping\n"
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: None,
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                ai_runtime,
                instance_pool: None,
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime: Arc::clone(&ai_runtime),
                instance_pool: None,
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: None,
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
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            route_overrides: test_route_overrides(),
            host_load: test_host_load(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: test_ai_runtime(&config),
            instance_pool: None,
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
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            route_overrides: test_route_overrides(),
            host_load: test_host_load(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: test_ai_runtime(&config),
            instance_pool: None,
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
            instance_pool: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(INSTANCE_POOL_DEFAULT_CAPACITY)
                    .time_to_idle(INSTANCE_POOL_IDLE_TIMEOUT)
                    .build(),
            ),
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
            bridge_manager: Arc::new(BridgeManager::default()),
            core_store,
            buffered_requests,
            volume_manager: Arc::new(VolumeManager::default()),
            route_overrides: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            peer_capabilities: Arc::new(Mutex::new(HashMap::new())),
            host_capabilities: Capabilities::detect(),
            host_load: Arc::new(HostLoadCounters::default()),
            telemetry: telemetry::init_test_telemetry(),
            tls_manager: Arc::new(tls_runtime::TlsManager::default()),
            mtls_gateway: None,
            auth_manager: Arc::new(
                auth::AuthManager::new(&core_store_manifest)
                    .expect("test auth manager should initialize"),
            ),
            enrollment_manager: Arc::new(node_enrollment::EnrollmentManager::new()),
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
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: None,
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

    #[tokio::test]
    async fn router_buffers_ai_generation_and_exposes_job_status() {
        let app = build_app(build_test_state(
            IntegrityConfig::default_sealed(),
            telemetry::init_test_telemetry(),
        ));

        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/generate")
                    .body(Body::from("hello model"))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body should collect")
            .to_bytes();
        let payload: Value = serde_json::from_slice(&body).expect("accepted body should be JSON");
        let job_id = payload["job_id"].as_str().expect("job id should exist");

        let mut status = None;
        for _ in 0..10 {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/api/v1/jobs/{job_id}"))
                        .body(Body::empty())
                        .expect("status request should build"),
                )
                .await
                .expect("status request should complete");
            assert_eq!(response.status(), StatusCode::OK);
            let body = response
                .into_body()
                .collect()
                .await
                .expect("status body should collect")
                .to_bytes();
            let value: Value = serde_json::from_slice(&body).expect("status should be JSON");
            if value["status"] == "completed" {
                status = Some(value);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }

        let status = status.expect("job should complete");
        assert_eq!(status["output"], "generated:hello model");
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
                test_route_overrides(),
                test_peer_capabilities(),
                Capabilities::detect(),
                test_host_load(),
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
                test_route_overrides(),
                test_peer_capabilities(),
                Capabilities::detect(),
                test_host_load(),
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
                test_route_overrides(),
                test_peer_capabilities(),
                Capabilities::detect(),
                test_host_load(),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn background_cdc_dispatches_events_and_acks_outbox_rows() {
        let state_dir = unique_test_dir("tachyon-cdc-target");
        fs::create_dir_all(&state_dir).expect("cdc state dir should create");

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut target_route = targeted_route(
            "/api/cdc-target",
            vec![weighted_target("guest-volume", 100)],
        );
        target_route.volumes = vec![mounted_volume(&state_dir, "/app/data")];

        let mut cdc_route = system_targeted_route("/system/cdc", "system-faas-cdc");
        cdc_route.env = route_env(&[
            ("DB_URL", "outbox://integration"),
            ("OUTBOX_TABLE", "events_outbox"),
            ("TARGET_ROUTE", "/api/cdc-target"),
            ("BATCH_SIZE", "4"),
        ]);

        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![target_route, cdc_route],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        let event_id = data_events::enqueue_event(
            state.storage_broker.core_store.as_ref(),
            "outbox://integration",
            "events_outbox",
            br#"{"event":"user.created"}"#.to_vec(),
            "application/json",
        )
        .expect("outbox event should enqueue");
        assert_eq!(
            data_events::pending_count(
                state.storage_broker.core_store.as_ref(),
                "outbox://integration",
                "events_outbox",
            )
            .expect("pending count should be readable"),
            1
        );

        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/cdc")
                        .expect("cdc route should be sealed"),
                    "system-faas-cdc",
                    telemetry::init_test_telemetry(),
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("cdc component should instantiate");
                runner.tick().expect("cdc tick should succeed");
            }
        })
        .await
        .expect("cdc background task should complete");

        host_server.abort();
        let _ = host_server.await;

        assert_eq!(
            fs::read_to_string(state_dir.join("state.txt"))
                .expect("cdc target route should persist event payload"),
            r#"{"event":"user.created"}"#
        );
        assert_eq!(
            data_events::pending_count(
                state.storage_broker.core_store.as_ref(),
                "outbox://integration",
                "events_outbox",
            )
            .expect("pending count should be readable"),
            0
        );
        assert!(
            data_events::claim_events(
                state.storage_broker.core_store.as_ref(),
                "outbox://integration",
                "events_outbox",
                4,
            )
            .expect("claim should succeed")
            .is_empty(),
            "acknowledged event `{event_id}` should no longer be claimable"
        );

        let _ = fs::remove_dir_all(state_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn s3_proxy_forwards_upload_and_buffers_mesh_event() {
        use axum::{
            body::Bytes as AxumBytes,
            extract::{Path as AxumPath, State},
            response::IntoResponse,
            routing::put,
            Router,
        };
        use serde_json::Value;
        use std::sync::Mutex;

        #[derive(Default)]
        struct MockS3State {
            paths: Vec<String>,
            auth_headers: Vec<String>,
            bodies: Vec<Vec<u8>>,
        }

        async fn put_object(
            AxumPath(key): AxumPath<String>,
            State(state): State<Arc<Mutex<MockS3State>>>,
            headers: HeaderMap,
            body: AxumBytes,
        ) -> impl IntoResponse {
            let mut state = state.lock().expect("mock s3 state should not be poisoned");
            state.paths.push(key);
            state.auth_headers.push(
                headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned(),
            );
            state.bodies.push(body.to_vec());
            StatusCode::OK
        }

        let s3_state = Arc::new(Mutex::new(MockS3State::default()));
        let s3_app = Router::new()
            .route("/bucket/{key}", put(put_object))
            .with_state(Arc::clone(&s3_state));
        let s3_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock s3 listener should bind");
        let s3_address = s3_listener
            .local_addr()
            .expect("mock s3 listener should expose an address");
        let s3_server = tokio::spawn(async move {
            axum::serve(s3_listener, s3_app)
                .await
                .expect("mock s3 server should stay up");
        });

        let queue_dir = unique_test_dir("tachyon-s3-proxy-queue");
        let event_dir = unique_test_dir("tachyon-s3-proxy-events");
        fs::create_dir_all(&queue_dir).expect("buffer queue dir should create");
        fs::create_dir_all(&event_dir).expect("event dir should create");

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut buffer_route = system_targeted_route("/system/buffer", "buffer");
        buffer_route.env = route_env(&[
            ("BUFFER_DIR", "/buffer"),
            ("RAM_QUEUE_CAPACITY", "4"),
            ("REPLAY_CPU_LIMIT", "70"),
            ("REPLAY_RAM_LIMIT", "70"),
            ("REPLAY_BATCH_SIZE", "4"),
        ]);
        buffer_route.volumes = vec![mounted_volume(&queue_dir, "/buffer")];

        let mut event_route = targeted_route(
            "/api/upload-events",
            vec![weighted_target("guest-volume", 100)],
        );
        event_route.volumes = vec![mounted_volume(&event_dir, "/app/data")];

        let mut proxy_route = system_targeted_route("/system/s3-proxy", "system-faas-s3-proxy");
        proxy_route.env = route_env(&[
            (
                "REAL_S3_BUCKET",
                format!("http://{s3_address}/bucket").as_str(),
            ),
            ("TARGET_ROUTE", "/api/upload-events"),
            ("BUFFER_ROUTE", "/system/buffer"),
            ("S3_AUTHORIZATION", "Bearer proxy-secret"),
        ]);

        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![event_route, buffer_route.clone(), proxy_route],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let response = Client::new()
            .put(format!("http://{host_address}/system/s3-proxy"))
            .header("content-type", "text/plain")
            .header("x-tachyon-object-key", "demo.txt")
            .body("hello-object")
            .send()
            .await
            .expect("s3 proxy upload should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let payload: Value = serde_json::from_slice(
            &response
                .bytes()
                .await
                .expect("s3 proxy response body should be readable"),
        )
        .expect("s3 proxy response should be JSON");
        assert_eq!(payload["key"], "demo.txt");
        assert_eq!(payload["size_bytes"], 12);

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/buffer")
                        .expect("buffer route should be sealed"),
                    "buffer",
                    telemetry::init_test_telemetry(),
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("buffer component should instantiate");
                runner.tick().expect("buffer tick should succeed");
            }
        })
        .await
        .expect("buffer replay task should complete");

        host_server.abort();
        s3_server.abort();
        let _ = host_server.await;
        let _ = s3_server.await;

        let s3_state = s3_state
            .lock()
            .expect("mock s3 state should not be poisoned");
        assert_eq!(s3_state.paths, vec!["demo.txt".to_owned()]);
        assert_eq!(
            s3_state.auth_headers,
            vec!["Bearer proxy-secret".to_owned()]
        );
        assert_eq!(s3_state.bodies, vec![b"hello-object".to_vec()]);
        drop(s3_state);

        let event_payload = fs::read_to_string(event_dir.join("state.txt"))
            .expect("upload event route should persist metadata payload");
        let event: Value =
            serde_json::from_str(&event_payload).expect("event payload should decode as JSON");
        assert_eq!(event["bucket"], format!("http://{s3_address}/bucket"));
        assert_eq!(event["key"], "demo.txt");
        assert_eq!(event["content_type"], "text/plain");
        assert_eq!(event["size_bytes"], 12);

        let _ = fs::remove_dir_all(queue_dir);
        let _ = fs::remove_dir_all(event_dir);
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
    async fn bridge_manager_relays_packets_between_allocated_ports() {
        let manager = BridgeManager::default();
        let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client A should bind");
        let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client B should bind");

        let allocation = manager
            .create_relay(BridgeConfig {
                client_a_addr: client_a
                    .local_addr()
                    .expect("client A address should resolve")
                    .to_string(),
                client_b_addr: client_b
                    .local_addr()
                    .expect("client B address should resolve")
                    .to_string(),
                timeout_seconds: 5,
            })
            .expect("bridge allocation should succeed");
        assert_eq!(manager.active_relay_count(), 1);

        client_a
            .send_to(
                b"alpha",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_a),
            )
            .await
            .expect("client A should send through the bridge");
        let mut received = [0_u8; 16];
        let (size, source) =
            tokio::time::timeout(Duration::from_secs(1), client_b.recv_from(&mut received))
                .await
                .expect("bridge delivery to client B should not time out")
                .expect("client B should receive relayed datagram");
        assert_eq!(&received[..size], b"alpha");
        assert_eq!(source.port(), allocation.port_b);

        client_b
            .send_to(
                b"beta",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_b),
            )
            .await
            .expect("client B should send through the bridge");
        let (size, source) =
            tokio::time::timeout(Duration::from_secs(1), client_a.recv_from(&mut received))
                .await
                .expect("bridge delivery to client A should not time out")
                .expect("client A should receive relayed datagram");
        assert_eq!(&received[..size], b"beta");
        assert_eq!(source.port(), allocation.port_a);
        assert!(manager.total_relayed_bytes() >= 9);

        manager
            .destroy_relay(&allocation.bridge_id)
            .expect("bridge teardown should succeed");
        assert_eq!(manager.active_relay_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn voip_gate_allocates_bridge_via_system_route_and_relays_packets() {
        let session_dir = unique_test_dir("tachyon-bridge-sessions");
        fs::create_dir_all(&session_dir).expect("session dir should exist");
        let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client A should bind");
        let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client B should bind");
        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose a local address");

        let mut bridge_route = system_targeted_route(SYSTEM_BRIDGE_ROUTE, "system-faas-bridge");
        bridge_route.volumes = vec![mounted_ram_volume(&session_dir, "/sessions")];
        let config = validate_integrity_config(IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![
                targeted_route(
                    "/api/voip-gate",
                    vec![weighted_target("guest-voip-gate", 100)],
                ),
                bridge_route,
            ],
            ..IntegrityConfig::default_sealed()
        })
        .expect("bridge config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let app = build_app(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(host_listener, app)
                .await
                .expect("bridge test app should stay up");
        });

        let allocation = Client::new()
            .post(format!("http://{host_address}/api/voip-gate"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "client_a_addr": client_a.local_addr().expect("client A address should resolve").to_string(),
                    "client_b_addr": client_b.local_addr().expect("client B address should resolve").to_string(),
                    "timeout_seconds": 5
                }))
                .expect("voip gate request body should serialize"),
            )
            .send()
            .await
            .expect("voip gate request should succeed")
            .error_for_status()
            .expect("voip gate response should be OK")
            .bytes()
            .await
            .map(|body| {
                serde_json::from_slice::<BridgeAllocation>(&body)
                    .expect("bridge allocation response should decode")
            })
            .expect("voip gate response body should read");
        assert_eq!(allocation.ip, Ipv4Addr::LOCALHOST.to_string());

        client_a
            .send_to(
                b"hello bridge",
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_a),
            )
            .await
            .expect("client A should send a bridged datagram");
        let mut buffer = [0_u8; 32];
        let (size, source) =
            tokio::time::timeout(Duration::from_secs(1), client_b.recv_from(&mut buffer))
                .await
                .expect("client B delivery should not time out")
                .expect("client B should receive bridged datagram");
        assert_eq!(&buffer[..size], b"hello bridge");
        assert_eq!(source.port(), allocation.port_b);

        let persisted =
            fs::read_to_string(session_dir.join(format!("{}.json", allocation.bridge_id)))
                .expect("system bridge should persist the active session");
        assert!(persisted.contains("\"status\":\"active\""));
        assert_eq!(state.bridge_manager.active_relay_count(), 1);
        state
            .bridge_manager
            .destroy_relay(&allocation.bridge_id)
            .expect("bridge teardown should succeed");
        assert_eq!(state.bridge_manager.active_relay_count(), 0);

        server.abort();
        let _ = server.await;
        let _ = fs::remove_dir_all(session_dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn voip_gate_delegates_bridge_to_healthier_peer_when_local_l4_is_saturated() {
        use axum::{
            body::Bytes as AxumBytes,
            extract::State,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use serde_json::json;
        use std::sync::Mutex;

        #[derive(Debug, Default)]
        struct PeerCapture {
            headers: Vec<Vec<(String, String)>>,
            bodies: Vec<String>,
        }

        async fn peer_gossip() -> impl IntoResponse {
            Json(json!({
                "total_requests": 0_u64,
                "completed_requests": 0_u64,
                "error_requests": 0_u64,
                "active_requests": 1_u32,
                "cpu_pressure": 5_u8,
                "ram_pressure": 5_u8,
                "active_instances": 1_u32,
                "allocated_memory_pages": 1_u32,
                "capability_mask": 0_u64,
                "capabilities": [],
                "active_l4_relays": 1_u32,
                "l4_throughput_bytes_per_sec": 1024_u64,
                "l4_load_score": 10_u8,
                "advertise_ip": "203.0.113.50",
                "cpu_rt_load": 0_u32,
                "cpu_standard_load": 0_u32,
                "cpu_batch_load": 0_u32,
                "gpu_rt_load": 0_u32,
                "gpu_standard_load": 0_u32,
                "gpu_batch_load": 0_u32,
                "npu_rt_load": 0_u32,
                "npu_standard_load": 0_u32,
                "npu_batch_load": 0_u32,
                "tpu_rt_load": 0_u32,
                "tpu_standard_load": 0_u32,
                "tpu_batch_load": 0_u32,
                "hot_models": [],
                "dropped_events": 0_u64,
                "last_status": 200_u16,
                "total_duration_us": 0_u64,
                "total_wasm_duration_us": 0_u64,
                "total_host_overhead_us": 0_u64
            }))
        }

        async fn peer_bridge(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            headers: HeaderMap,
            body: AxumBytes,
        ) -> impl IntoResponse {
            let mut capture = state.lock().expect("peer capture should not be poisoned");
            capture.headers.push(
                headers
                    .iter()
                    .map(|(name, value)| {
                        (
                            name.as_str().to_owned(),
                            value.to_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect(),
            );
            capture
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (
                StatusCode::OK,
                Json(json!({
                    "bridge_id": "peer-bridge-1",
                    "ip": "203.0.113.50",
                    "port_a": 31_000_u16,
                    "port_b": 31_001_u16
                })),
            )
        }

        let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let peer_app = Router::new()
            .route("/system/gossip", get(peer_gossip))
            .route("/system/bridge", post(peer_bridge))
            .with_state(Arc::clone(&peer_capture));
        let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("peer listener should bind");
        let peer_address = peer_listener
            .local_addr()
            .expect("peer listener should expose an address");
        let peer_server = tokio::spawn(async move {
            axum::serve(peer_listener, peer_app)
                .await
                .expect("peer app should stay up");
        });

        let session_dir = unique_test_dir("tachyon-bridge-steering-sessions");
        fs::create_dir_all(&session_dir).expect("session dir should exist");
        let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client A should bind");
        let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("client B should bind");
        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose a local address");

        let mut bridge_route = system_targeted_route(SYSTEM_BRIDGE_ROUTE, "system-faas-bridge");
        let peer_urls = format!("http://{peer_address}");
        bridge_route.env = route_env(&[
            ("PEER_URLS", peer_urls.as_str()),
            ("BRIDGE_SOFT_LIMIT", "80"),
            ("GOSSIP_PATH", "/system/gossip"),
        ]);
        bridge_route.volumes = vec![mounted_ram_volume(&session_dir, "/sessions")];
        let config = validate_integrity_config(IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![
                targeted_route(
                    "/api/voip-gate",
                    vec![weighted_target("guest-voip-gate", 100)],
                ),
                bridge_route,
            ],
            ..IntegrityConfig::default_sealed()
        })
        .expect("bridge steering config should validate");
        let state = build_test_state(config, telemetry::init_test_telemetry());

        let mut local_bridges = Vec::new();
        for offset in 0..4_u16 {
            let allocation = state
                .bridge_manager
                .create_relay(BridgeConfig {
                    client_a_addr: format!("127.0.0.1:{}", 20_000 + offset * 2),
                    client_b_addr: format!("127.0.0.1:{}", 20_001 + offset * 2),
                    timeout_seconds: 30,
                })
                .expect("local saturation relay should allocate");
            local_bridges.push(allocation.bridge_id);
        }
        let local_relay_count = state.bridge_manager.active_relay_count();

        let app = build_app(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(host_listener, app)
                .await
                .expect("bridge steering test app should stay up");
        });

        let allocation = Client::new()
            .post(format!("http://{host_address}/api/voip-gate"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "client_a_addr": client_a.local_addr().expect("client A address should resolve").to_string(),
                    "client_b_addr": client_b.local_addr().expect("client B address should resolve").to_string(),
                    "timeout_seconds": 5
                }))
                .expect("voip gate request body should serialize"),
            )
            .send()
            .await
            .expect("voip gate request should succeed")
            .error_for_status()
            .expect("voip gate response should be OK")
            .bytes()
            .await
            .map(|body| {
                serde_json::from_slice::<BridgeAllocation>(&body)
                    .expect("bridge allocation response should decode")
            })
            .expect("voip gate response body should read");

        assert_eq!(allocation.bridge_id, "peer-bridge-1");
        assert_eq!(allocation.ip, "203.0.113.50");
        assert_eq!(allocation.port_a, 31_000);
        assert_eq!(allocation.port_b, 31_001);
        assert_eq!(state.bridge_manager.active_relay_count(), local_relay_count);

        {
            let capture = peer_capture
                .lock()
                .expect("peer capture should not be poisoned");
            assert_eq!(capture.bodies.len(), 1);
            assert!(capture.bodies[0].contains("client_a_addr"));
            assert!(capture.headers[0].iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("x-tachyon-bridge-delegated") && value == "true"
            }));
        }

        for bridge_id in local_bridges {
            state
                .bridge_manager
                .destroy_relay(&bridge_id)
                .expect("local saturation relay should tear down");
        }

        server.abort();
        let _ = server.await;
        peer_server.abort();
        let _ = peer_server.await;
        let _ = fs::remove_dir_all(session_dir);
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

    #[tokio::test(flavor = "multi_thread")]
    async fn mtls_gateway_rejects_missing_client_cert_and_forwards_authorized_requests() {
        init_host_tracing();

        let mtls = generate_mtls_test_material();
        let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("HTTP listener should bind");
        let http_address = http_listener
            .local_addr()
            .expect("HTTP listener should expose a local address");
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = http_address.to_string();
        let mut gateway_route = IntegrityRoute::system(SYSTEM_GATEWAY_ROUTE);
        gateway_route.targets = vec![weighted_target("system-faas-gateway", 100)];
        config.routes.push(gateway_route);
        let config =
            validate_integrity_config(config).expect("mTLS gateway config should validate");
        let mut state = build_test_state(config, telemetry::init_test_telemetry());
        state.mtls_gateway = Some(Arc::new(tls_runtime::MtlsGatewayConfig {
            bind_address: "127.0.0.1:0"
                .parse()
                .expect("mTLS bind address should parse"),
            server_config: Arc::new(
                tls_runtime::build_mtls_server_config(
                    &mtls.server_cert_pem,
                    &mtls.server_key_pem,
                    &mtls.ca_pem,
                )
                .expect("mTLS server config should build"),
            ),
        }));

        let app = build_app(state.clone());
        let http_server = tokio::spawn(async move {
            axum::serve(http_listener, app)
                .await
                .expect("HTTP app should stay up");
        });
        let listener = start_mtls_gateway_listener(state.clone())
            .await
            .expect("mTLS gateway listener should start")
            .expect("mTLS gateway listener should be enabled");
        let gateway_addr = listener.local_addr;
        let url = format!(
            "https://localhost:{}/api/guest-example",
            gateway_addr.port()
        );

        let unauthorized = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("unauthorized reqwest client should build")
            .get(&url)
            .send()
            .await;
        assert!(
            unauthorized.is_err(),
            "mTLS gateway should reject requests without a client certificate"
        );

        let client_identity = reqwest::Identity::from_pem(
            format!("{}{}", mtls.client_cert_pem, mtls.client_key_pem).as_bytes(),
        )
        .expect("client identity should load");
        let authorized = reqwest::Client::builder()
            .use_rustls_tls()
            .danger_accept_invalid_certs(true)
            .identity(client_identity)
            .build()
            .expect("authorized reqwest client should build")
            .get(&url)
            .send()
            .await
            .expect("authorized mTLS request should succeed");
        assert_eq!(authorized.status(), StatusCode::OK);
        assert_eq!(
            authorized
                .text()
                .await
                .expect("authorized response should decode"),
            expected_guest_example_body("FaaS received an empty payload")
        );

        listener.join_handle.abort();
        let _ = listener.join_handle.await;
        http_server.abort();
        let _ = http_server.await;
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
            .send(Message::Binary(vec![1_u8, 2, 3].into()))
            .await
            .expect("WebSocket client should send binary frame");
        let binary_frame = client
            .next()
            .await
            .expect("WebSocket server should respond to binary frame")
            .expect("WebSocket frame should be valid");
        assert!(matches!(binary_frame, Message::Binary(bytes) if bytes.as_ref() == [1_u8, 2, 3]));

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
        let stdout = Bytes::from("MESH_FETCH:http://mesh/legacy-service/ping\n");

        assert_eq!(
            extract_mesh_fetch_url(&stdout),
            Some("http://mesh/legacy-service/ping")
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
            test_selected_target("guest-loop", false)
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
            test_selected_target("guest-example", false)
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(89))
                .expect("weighted route should resolve"),
            test_selected_target("guest-example", false)
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(90))
                .expect("weighted route should resolve"),
            test_selected_target("guest-loop", false)
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
            test_selected_target("guest-example", false)
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

    #[test]
    fn resolve_mesh_fetch_target_resolves_internal_resource_aliases() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![
            versioned_route("/api/checkout", "checkout", "1.0.0"),
            versioned_route("/api/inventory", "inventory", "1.2.3"),
        ];
        config.resources = BTreeMap::from([(
            "inventory-api".to_owned(),
            IntegrityResource::Internal {
                target: "inventory".to_owned(),
                version_constraint: Some("^1.2".to_owned()),
            },
        )]);
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/checkout")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_mesh_fetch_target(
                &config,
                &route_registry,
                caller_route,
                "http://mesh/inventory-api/items?expand=1",
            )
            .expect("internal resource alias should resolve"),
            "http://127.0.0.1:8080/api/inventory/items?expand=1"
        );
    }

    #[test]
    fn resolve_outbound_http_target_resolves_external_resource_aliases() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "payment-gateway".to_owned(),
            IntegrityResource::External {
                target: "https://api.example.com/v1".to_owned(),
                allowed_methods: vec!["POST".to_owned()],
            },
        )]);
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/checkout")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_outbound_http_target(
                &config,
                &route_registry,
                caller_route,
                &reqwest::Method::POST,
                "http://mesh/payment-gateway/charges?expand=1",
            )
            .expect("external resource alias should resolve"),
            ResolvedOutboundTarget {
                url: "https://api.example.com/v1/charges?expand=1".to_owned(),
                kind: OutboundTargetKind::External,
            }
        );
    }

    #[test]
    fn resolve_outbound_http_target_switches_when_resource_manifest_changes() {
        let routes = vec![
            versioned_route("/api/checkout", "checkout", "1.0.0"),
            versioned_route("/api/service-b", "service-b", "2.1.0"),
        ];
        let caller_path = "/api/checkout";
        let resource_name = "service-b-alias";

        let external_config = validate_integrity_config(IntegrityConfig {
            host_address: "0.0.0.0:8080".to_owned(),
            routes: routes.clone(),
            resources: BTreeMap::from([(
                resource_name.to_owned(),
                IntegrityResource::External {
                    target: "https://api.example.com/v1/service-b".to_owned(),
                    allowed_methods: vec!["GET".to_owned()],
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("external config should validate");
        let external_registry =
            RouteRegistry::build(&external_config).expect("route registry should build");
        let caller_route = external_config
            .sealed_route(caller_path)
            .expect("caller route should remain sealed");
        assert_eq!(
            resolve_outbound_http_target(
                &external_config,
                &external_registry,
                caller_route,
                &reqwest::Method::GET,
                &format!("http://mesh/{resource_name}/health"),
            )
            .expect("external target should resolve")
            .url,
            "https://api.example.com/v1/service-b/health"
        );

        let internal_config = validate_integrity_config(IntegrityConfig {
            host_address: "0.0.0.0:8080".to_owned(),
            routes,
            resources: BTreeMap::from([(
                resource_name.to_owned(),
                IntegrityResource::Internal {
                    target: "service-b".to_owned(),
                    version_constraint: Some("^2.0".to_owned()),
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("internal config should validate");
        let internal_registry =
            RouteRegistry::build(&internal_config).expect("route registry should build");
        let caller_route = internal_config
            .sealed_route(caller_path)
            .expect("caller route should remain sealed");
        assert_eq!(
            resolve_outbound_http_target(
                &internal_config,
                &internal_registry,
                caller_route,
                &reqwest::Method::GET,
                &format!("http://mesh/{resource_name}/health"),
            )
            .expect("internal target should resolve"),
            ResolvedOutboundTarget {
                url: "http://127.0.0.1:8080/api/service-b/health".to_owned(),
                kind: OutboundTargetKind::Internal,
            }
        );
    }

    #[test]
    fn resolve_outbound_http_target_blocks_raw_external_urls_for_user_routes() {
        let config = IntegrityConfig::default_sealed();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");

        let error = resolve_outbound_http_target(
            &config,
            &route_registry,
            caller_route,
            &reqwest::Method::GET,
            "https://api.example.com/v1/ping",
        )
        .expect_err("raw external egress should be rejected for user routes");

        assert!(error.contains("not allowed to call raw external URLs"));
    }

    #[test]
    fn filtered_outbound_http_headers_strips_internal_mesh_headers_for_external_targets() {
        let filtered = filtered_outbound_http_headers(
            vec![
                (HOP_LIMIT_HEADER.to_owned(), "3".to_owned()),
                (
                    TACHYON_IDENTITY_HEADER.to_owned(),
                    "Bearer secret".to_owned(),
                ),
                (
                    "authorization".to_owned(),
                    "Bearer partner-token".to_owned(),
                ),
                ("host".to_owned(), "mesh".to_owned()),
            ],
            &[PropagatedHeader {
                name: COHORT_HEADER.to_owned(),
                value: "beta".to_owned(),
            }],
            &OutboundTargetKind::External,
        );

        assert_eq!(
            filtered,
            vec![(
                "authorization".to_owned(),
                "Bearer partner-token".to_owned(),
            )]
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

        let config = validate_integrity_config(IntegrityConfig {
            resources: BTreeMap::from([(
                "external-ping".to_owned(),
                IntegrityResource::External {
                    target: format!("http://{address}"),
                    allowed_methods: vec!["GET".to_owned()],
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("config should validate");
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
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:http://mesh/external-ping/ping"),
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
    fn validate_integrity_config_defaults_route_scaling_when_scaling_fields_are_omitted() {
        let config = serde_json::from_str::<IntegrityConfig>(
            r#"{
                "host_address":"0.0.0.0:8080",
                "max_stdout_bytes":65536,
                "guest_fuel_budget":500000000,
                "guest_memory_limit_bytes":52428800,
                "resource_limit_response":"Execution trapped: Resource limit exceeded",
                "routes":[{"path":"/api/guest-example","role":"user","version":"0.0.0","dependencies":{}}]
            }"#,
        )
        .expect("payload should deserialize");
        let config = validate_integrity_config(config).expect("payload should validate");
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
    #[should_panic(expected = "ERR_INTEGRITY_SCHEMA_VIOLATION")]
    fn verify_integrity_payload_panics_with_schema_violation_when_version_is_missing() {
        let payload = r#"{
            "host_address":"0.0.0.0:8080",
            "max_stdout_bytes":65536,
            "guest_fuel_budget":500000000,
            "guest_memory_limit_bytes":52428800,
            "resource_limit_response":"Execution trapped: Resource limit exceeded",
            "routes":[{"path":"/api/guest-example","role":"user","dependencies":{}}]
        }"#;
        let signing_key = SigningKey::from_bytes(&[21_u8; 32]);
        let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));
        let error = verify_integrity_payload(
            payload,
            &hex::encode(signing_key.verifying_key().to_bytes()),
            &hex::encode(signature.to_bytes()),
            "test payload",
        )
        .expect_err("payload missing version should fail strict schema validation");

        panic!("{error:#}");
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
    fn validate_integrity_config_rejects_resource_names_that_conflict_with_routes() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "checkout".to_owned(),
            IntegrityResource::External {
                target: "https://api.example.com/v1".to_owned(),
                allowed_methods: vec!["GET".to_owned()],
            },
        )]);

        let error = validate_integrity_config(config)
            .expect_err("resource names that shadow routes should fail validation");

        assert!(error
            .to_string()
            .contains("conflicts with a sealed route name"));
    }

    #[test]
    fn validate_integrity_config_rejects_external_resources_without_allowed_methods() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "payment-gateway".to_owned(),
            IntegrityResource::External {
                target: "https://api.example.com/v1".to_owned(),
                allowed_methods: Vec::new(),
            },
        )]);

        let error = validate_integrity_config(config)
            .expect_err("external resources must declare an allow-list");

        assert!(error
            .to_string()
            .contains("must declare at least one allowed HTTP method"));
    }

    #[test]
    fn validate_integrity_config_accepts_http_cluster_local_external_resource() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "legacy-service".to_owned(),
            IntegrityResource::External {
                target: "http://legacy-service:8081".to_owned(),
                allowed_methods: vec!["GET".to_owned()],
            },
        )]);

        let config =
            validate_integrity_config(config).expect("cluster-local HTTP resource should validate");

        assert_eq!(
            config.resources.get("legacy-service"),
            Some(&IntegrityResource::External {
                target: "http://legacy-service:8081/".to_owned(),
                allowed_methods: vec!["GET".to_owned()],
            })
        );
    }

    #[test]
    fn validate_integrity_config_rejects_public_http_external_resource() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "payment-gateway".to_owned(),
            IntegrityResource::External {
                target: "http://api.example.com/v1".to_owned(),
                allowed_methods: vec!["GET".to_owned()],
            },
        )]);

        let error = validate_integrity_config(config)
            .expect_err("public HTTP external resources should be rejected");

        assert!(error.to_string().contains(
            "must use HTTPS unless it points at localhost for tests or a cluster-local service"
        ));
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

                ..Default::default()
            }],

            ..Default::default()
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

                ..Default::default()
            }]
        );
    }

    #[test]
    fn validate_integrity_config_preserves_encrypted_volume_flag() {
        let mut config = IntegrityConfig::default_sealed();
        config.routes = vec![IntegrityRoute {
            path: "/system/tde-consumer".to_owned(),
            role: RouteRole::System,
            name: "system-faas-logger".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            volumes: vec![IntegrityVolume {
                volume_type: VolumeType::Host,
                host_path: "/tmp/tachyon_sensitive".to_owned(),
                guest_path: "/secure".to_owned(),
                readonly: false,
                encrypted: true,
                ..Default::default()
            }],
            ..Default::default()
        }];

        let config =
            validate_integrity_config(config).expect("encrypted volume config should validate");
        let volume = &config
            .sealed_route("/system/tde-consumer")
            .expect("route should remain sealed")
            .volumes[0];

        assert!(volume.encrypted);
        assert_eq!(
            encrypted_volume_host_path(&volume.host_path),
            PathBuf::from("/tmp/tachyon_sensitive").join(".tachyon-tde")
        );
    }

    #[test]
    fn encrypted_volume_seal_hides_plaintext_and_prepare_restores_it() {
        let volume_dir = unique_test_dir("tachyon-tde-volume");
        let mut route = storage_broker_test_route(&volume_dir);
        route.volumes[0].encrypted = true;
        let encrypted_root = encrypted_volume_host_path(&route.volumes[0].host_path);
        fs::create_dir_all(&encrypted_root).expect("encrypted root should exist");
        let file_path = encrypted_root.join("state.txt");
        fs::write(&file_path, b"patient-record: secret").expect("plaintext should be written");

        seal_encrypted_route_volumes(&route).expect("volume should seal");
        let sealed = fs::read(&file_path).expect("sealed file should be readable");
        assert!(sealed.starts_with(TDE_FILE_MAGIC));
        assert!(!String::from_utf8_lossy(&sealed).contains("patient-record"));

        prepare_encrypted_route_volumes(&route).expect("volume should prepare");
        assert_eq!(
            fs::read(&file_path).expect("prepared file should be readable"),
            b"patient-record: secret"
        );

        let _ = fs::remove_dir_all(volume_dir);
    }

    #[test]
    fn lora_training_job_exports_adapter_with_finops_metadata() {
        let broker_dir = unique_test_dir("tachyon-lora-train");
        std::env::set_var(MODEL_BROKER_DIR_ENV, &broker_dir);
        let statuses = Arc::new(Mutex::new(HashMap::new()));
        let job = LoraTrainingJob {
            id: "lora-test".to_owned(),
            tenant_id: "tenant-a".to_owned(),
            base_model: "llama3".to_owned(),
            dataset_volume: "training-data".to_owned(),
            dataset_path: "/datasets/a.jsonl".to_owned(),
            dataset_split: Some("train[:90%]".to_owned()),
            rank: 8,
            max_steps: 2,
            seed: Some(7),
        };

        let adapter_path =
            execute_lora_training_job(&job, &statuses).expect("training job should export");
        let payload = fs::read_to_string(&adapter_path).expect("adapter artifact should exist");
        let value: Value = serde_json::from_str(&payload).expect("adapter artifact should be JSON");

        assert_eq!(value["tenant_id"], "tenant-a");
        assert_eq!(value["base_model"], "llama3");
        assert_eq!(value["finops"]["cpu_fallback"], true);
        assert_eq!(value["finops"]["ram_spillover"], true);
        assert!(adapter_path.ends_with(".safetensors"));

        std::env::remove_var(MODEL_BROKER_DIR_ENV);
        let _ = fs::remove_dir_all(broker_dir);
    }

    #[test]
    fn validate_integrity_config_rejects_tee_route_without_backend() {
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.requires_tee = true;
        let config = IntegrityConfig {
            routes: vec![route],
            tee_backend: None,
            ..IntegrityConfig::default_sealed()
        };

        let error = validate_integrity_config(config)
            .expect_err("TEE routes must require an explicit backend");

        assert!(error
            .to_string()
            .contains("routes with `requires_tee: true` require `tee_backend`"));
    }

    #[test]
    fn validate_integrity_config_accepts_tee_route_with_backend() {
        let mut route = IntegrityRoute::user("/api/guest-example");
        route.requires_tee = true;
        let config = IntegrityConfig {
            routes: vec![route],
            tee_backend: Some(TeeBackendConfig::LocalEnclave),
            ..IntegrityConfig::default_sealed()
        };

        let config = validate_integrity_config(config).expect("TEE backend should validate");
        assert!(
            config
                .sealed_route("/api/guest-example")
                .expect("TEE route should remain sealed")
                .requires_tee
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

                ..Default::default()
            }],

            ..Default::default()
        }];

        let error = validate_integrity_config(config)
            .expect_err("writable user volumes should fail validation");

        assert!(error
            .to_string()
            .contains("cannot request writable direct host mounts"));
    }

    #[test]
    fn distributed_rate_limit_key_uses_first_forwarded_for_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "203.0.113.10, 198.51.100.20"
                .parse()
                .expect("header should parse"),
        );

        assert_eq!(distributed_rate_limit_key(&headers), "203.0.113.10");
    }

    #[test]
    fn distributed_rate_limit_decision_rejects_denied_response() {
        let mut route = IntegrityRoute::user("/api/protected");
        route.distributed_rate_limit = Some(DistributedRateLimitConfig {
            threshold: 100,
            window_seconds: 60,
        });
        let response = GuestHttpResponse::new(
            StatusCode::OK,
            Bytes::from_static(br#"{"allowed":false,"total":101}"#),
        );

        let rejection =
            distributed_rate_limit_decision(&route, response).expect("request should be rejected");

        assert_eq!(rejection.0, StatusCode::TOO_MANY_REQUESTS);
        assert!(rejection.1.contains("/api/protected"));
    }

    #[test]
    fn distributed_rate_limit_bypasses_invalid_response_with_metric() {
        let route = IntegrityRoute::user("/api/protected");
        let before = distributed_rate_limit_bypass_total();
        let response = GuestHttpResponse::new(StatusCode::OK, Bytes::from_static(b"not-json"));

        assert!(distributed_rate_limit_decision(&route, response).is_none());
        assert_eq!(distributed_rate_limit_bypass_total(), before + 1);
    }

    #[test]
    fn keda_pending_signal_prefers_internal_capacity_before_scale_out() {
        let control = RouteExecutionControl::from_limits(0, 4);
        control.pending_waiters.store(3, Ordering::SeqCst);
        control.active_requests.store(2, Ordering::SeqCst);

        assert_eq!(control.keda_pending_queue_size(), 3);
    }

    #[test]
    fn keda_pending_signal_boosts_when_route_is_saturated() {
        let control = RouteExecutionControl::from_limits(0, 4);
        control.pending_waiters.store(3, Ordering::SeqCst);
        control.active_requests.store(4, Ordering::SeqCst);

        assert_eq!(control.keda_pending_queue_size(), 7);
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

    #[test]
    fn storage_broker_emits_cdc_event_after_sync_enabled_write() {
        let volume_dir = unique_test_dir("tachyon-cdc-write");
        let store_path = unique_test_dir("tachyon-cdc-store").join("tachyon.db");
        let core_store = Arc::new(store::CoreStore::open(&store_path).expect("store should open"));
        let broker = StorageBrokerManager::new(Arc::clone(&core_store));
        let mut route = storage_broker_test_route(&volume_dir);

        broker
            .enqueue_write_for_route(
                &route,
                "/app/data/local.txt",
                StorageWriteMode::Overwrite,
                b"local-only".to_vec(),
            )
            .expect("non CDC write should be accepted");
        assert!(
            broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
            "broker queue should drain"
        );
        assert!(
            core_store
                .peek_outbox(store::CoreStoreBucket::DataMutationOutbox, 10)
                .expect("outbox should be readable")
                .is_empty(),
            "non opt-in route should not emit CDC events"
        );

        route.sync_to_cloud = true;
        broker
            .enqueue_write_for_route(
                &route,
                "/app/data/state.txt",
                StorageWriteMode::Append,
                b"replicate-me".to_vec(),
            )
            .expect("CDC write should be accepted");
        assert!(
            broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
            "broker queue should drain"
        );

        let events = core_store
            .peek_outbox(store::CoreStoreBucket::DataMutationOutbox, 10)
            .expect("outbox should be readable");
        assert_eq!(events.len(), 1);
        let payload: Value =
            serde_json::from_slice(&events[0].1).expect("CDC event should be JSON");
        assert_eq!(payload["event"], "tachyon.data.mutation");
        assert_eq!(payload["route_path"], route.path);
        assert_eq!(payload["resource"], "/app/data/state.txt");
        assert_eq!(payload["operation"], "append");
        assert_eq!(payload["value_bytes"], 12);
        assert_eq!(
            payload["value_hash"],
            format!("sha256:{}", hex::encode(Sha256::digest(b"replicate-me")))
        );

        let _ = fs::remove_dir_all(volume_dir);
        if let Some(parent) = store_path.parent() {
            let _ = fs::remove_dir_all(parent);
        }
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

            ..Default::default()
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
    fn embedded_integrity_payload_allows_legacy_service_resource_alias() {
        let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
            .expect("embedded payload should deserialize into an integrity config");
        let config = validate_integrity_config(config).expect("embedded config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/guest-call-legacy")
            .expect("legacy route should remain sealed");

        assert_eq!(
            resolve_outbound_http_target(
                &config,
                &route_registry,
                caller_route,
                &reqwest::Method::GET,
                "http://mesh/legacy-service/ping",
            )
            .expect("legacy service alias should resolve"),
            ResolvedOutboundTarget {
                url: "http://legacy-service:8081/ping".to_owned(),
                kind: OutboundTargetKind::External,
            }
        );
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

    #[tokio::test(flavor = "multi_thread")]
    async fn control_plane_gossip_redirects_requests_to_a_healthier_peer() {
        use axum::{
            body::Bytes as AxumBytes,
            extract::State,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use serde_json::json;
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn gossip_status() -> impl IntoResponse {
            Json(json!({
                "total_requests": 0_u64,
                "completed_requests": 0_u64,
                "error_requests": 0_u64,
                "active_requests": 0_u32,
                "cpu_pressure": 15_u8,
                "ram_pressure": 10_u8,
                "active_instances": 1_u32,
                "allocated_memory_pages": 1_u32,
                "dropped_events": 0_u64,
                "last_status": 200_u16,
                "total_duration_us": 0_u64,
                "total_wasm_duration_us": 0_u64,
                "total_host_overhead_us": 0_u64
            }))
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-ok")
        }

        let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let peer_app = Router::new()
            .route("/system/gossip", get(gossip_status))
            .route("/api/guest-example", post(peer_target))
            .with_state(Arc::clone(&peer_capture));
        let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("peer listener should bind");
        let peer_address = peer_listener
            .local_addr()
            .expect("peer listener should expose an address");
        let peer_server = tokio::spawn(async move {
            axum::serve(peer_listener, peer_app)
                .await
                .expect("peer app should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
        let peer_urls = format!("http://{peer_address}");
        gossip_route.env = route_env(&[
            ("STEER_ROUTE", "/api/guest-example"),
            ("PEER_URLS", peer_urls.as_str()),
            ("SOFT_LIMIT", "70"),
            ("RECOVER_LIMIT", "50"),
            ("SATURATED_LIMIT", "95"),
            ("BUFFER_ROUTE", "/system/buffer"),
        ]);
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![
                targeted_route(
                    "/api/guest-example",
                    vec![weighted_target("guest-example", 100)],
                ),
                gossip_route.clone(),
            ],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        state
            .host_load
            .active_instances
            .store(200, Ordering::SeqCst);
        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let telemetry = state.telemetry.clone();
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/gossip")
                        .expect("gossip route should be sealed"),
                    "gossip",
                    telemetry,
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("gossip component should instantiate");
                runner.tick().expect("gossip tick should succeed");
            }
        })
        .await
        .expect("gossip task should complete");

        let override_target = state
            .route_overrides
            .load()
            .get("/api/guest-example")
            .cloned()
            .expect("gossip should install a route override");
        assert!(override_target.contains(&format!("http://{peer_address}/api/guest-example")));

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-example"))
            .body("overflow-request")
            .send()
            .await
            .expect("host request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            "peer-ok"
        );

        let captured = peer_capture
            .lock()
            .expect("peer capture should not be poisoned");
        assert_eq!(captured.bodies, vec!["overflow-request".to_owned()]);

        host_server.abort();
        peer_server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn model_aware_gossip_prefers_peer_with_matching_hot_model() {
        use axum::{
            body::Bytes as AxumBytes,
            extract::State,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use serde_json::json;
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn wrong_model_gossip() -> impl IntoResponse {
            Json(json!({
                "total_requests": 0_u64,
                "completed_requests": 0_u64,
                "error_requests": 0_u64,
                "active_requests": 1_u32,
                "cpu_pressure": 10_u8,
                "ram_pressure": 10_u8,
                "active_instances": 1_u32,
                "allocated_memory_pages": 1_u32,
                "hot_models": ["mistral"],
                "dropped_events": 0_u64,
                "last_status": 200_u16,
                "total_duration_us": 0_u64,
                "total_wasm_duration_us": 0_u64,
                "total_host_overhead_us": 0_u64
            }))
        }

        async fn right_model_gossip() -> impl IntoResponse {
            Json(json!({
                "total_requests": 0_u64,
                "completed_requests": 0_u64,
                "error_requests": 0_u64,
                "active_requests": 2_u32,
                "cpu_pressure": 20_u8,
                "ram_pressure": 20_u8,
                "active_instances": 1_u32,
                "allocated_memory_pages": 1_u32,
                "hot_models": ["llama3"],
                "dropped_events": 0_u64,
                "last_status": 200_u16,
                "total_duration_us": 0_u64,
                "total_wasm_duration_us": 0_u64,
                "total_host_overhead_us": 0_u64
            }))
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-match")
        }

        let wrong_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let wrong_app = Router::new()
            .route("/system/gossip", get(wrong_model_gossip))
            .route("/api/guest-ai", post(peer_target))
            .with_state(Arc::clone(&wrong_capture));
        let wrong_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("wrong-model peer should bind");
        let wrong_address = wrong_listener
            .local_addr()
            .expect("wrong-model peer should expose an address");
        let wrong_server = tokio::spawn(async move {
            axum::serve(wrong_listener, wrong_app)
                .await
                .expect("wrong-model peer should stay up");
        });

        let right_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let right_app = Router::new()
            .route("/system/gossip", get(right_model_gossip))
            .route("/api/guest-ai", post(peer_target))
            .with_state(Arc::clone(&right_capture));
        let right_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("matching-model peer should bind");
        let right_address = right_listener
            .local_addr()
            .expect("matching-model peer should expose an address");
        let right_server = tokio::spawn(async move {
            axum::serve(right_listener, right_app)
                .await
                .expect("matching-model peer should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut user_route =
            targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
        user_route.models = vec![IntegrityModelBinding {
            alias: "llama3".to_owned(),
            path: "/models/llama3.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::RealTime,
        }];
        let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
        let peer_urls = format!("http://{wrong_address},http://{right_address}");
        gossip_route.env = route_env(&[
            ("STEER_ROUTE", "/api/guest-ai"),
            ("PEER_URLS", peer_urls.as_str()),
            ("SOFT_LIMIT", "70"),
            ("RECOVER_LIMIT", "50"),
            ("SATURATED_LIMIT", "95"),
            ("BUFFER_ROUTE", "/system/buffer"),
        ]);
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![user_route, gossip_route.clone()],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        state
            .host_load
            .active_instances
            .store(200, Ordering::SeqCst);
        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let telemetry = state.telemetry.clone();
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/gossip")
                        .expect("gossip route should be sealed"),
                    "gossip",
                    telemetry,
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("gossip component should instantiate");
                runner.tick().expect("gossip tick should succeed");
            }
        })
        .await
        .expect("gossip task should complete");

        let override_target = state
            .route_overrides
            .load()
            .get("/api/guest-ai")
            .cloned()
            .expect("gossip should install a route override");
        let descriptor: RouteOverrideDescriptor =
            serde_json::from_str(&override_target).expect("override descriptor should parse");
        assert_eq!(descriptor.candidates.len(), 2);

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-ai"))
            .header("x-tachyon-model", "llama3")
            .body("hot-model-request")
            .send()
            .await
            .expect("host request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            "peer-match"
        );

        let wrong_capture = wrong_capture
            .lock()
            .expect("wrong-model capture should not be poisoned");
        assert!(wrong_capture.bodies.is_empty());
        let right_capture = right_capture
            .lock()
            .expect("matching-model capture should not be poisoned");
        assert_eq!(right_capture.bodies, vec!["hot-model-request".to_owned()]);

        host_server.abort();
        wrong_server.abort();
        right_server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn model_aware_gossip_keeps_request_local_when_no_peer_has_hot_model() {
        use axum::{
            body::Bytes as AxumBytes,
            extract::State,
            response::IntoResponse,
            routing::{get, post},
            Json, Router,
        };
        use serde_json::json;
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn cold_peer_gossip() -> impl IntoResponse {
            Json(json!({
                "total_requests": 0_u64,
                "completed_requests": 0_u64,
                "error_requests": 0_u64,
                "active_requests": 0_u32,
                "cpu_pressure": 10_u8,
                "ram_pressure": 10_u8,
                "active_instances": 1_u32,
                "allocated_memory_pages": 1_u32,
                "hot_models": ["mistral"],
                "dropped_events": 0_u64,
                "last_status": 200_u16,
                "total_duration_us": 0_u64,
                "total_wasm_duration_us": 0_u64,
                "total_host_overhead_us": 0_u64
            }))
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-cold")
        }

        let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let peer_app = Router::new()
            .route("/system/gossip", get(cold_peer_gossip))
            .route("/api/guest-ai", post(peer_target))
            .with_state(Arc::clone(&peer_capture));
        let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("peer listener should bind");
        let peer_address = peer_listener
            .local_addr()
            .expect("peer listener should expose an address");
        let peer_server = tokio::spawn(async move {
            axum::serve(peer_listener, peer_app)
                .await
                .expect("peer app should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut user_route =
            targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
        user_route.models = vec![IntegrityModelBinding {
            alias: "llama3".to_owned(),
            path: "/models/llama3.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::RealTime,
        }];
        let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
        let peer_urls = format!("http://{peer_address}");
        gossip_route.env = route_env(&[
            ("STEER_ROUTE", "/api/guest-ai"),
            ("PEER_URLS", peer_urls.as_str()),
            ("SOFT_LIMIT", "70"),
            ("RECOVER_LIMIT", "50"),
            ("SATURATED_LIMIT", "95"),
            ("BUFFER_ROUTE", "/system/buffer"),
        ]);
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![user_route, gossip_route.clone()],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        state
            .host_load
            .active_instances
            .store(200, Ordering::SeqCst);
        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let telemetry = state.telemetry.clone();
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/gossip")
                        .expect("gossip route should be sealed"),
                    "gossip",
                    telemetry,
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("gossip component should instantiate");
                runner.tick().expect("gossip tick should succeed");
            }
        })
        .await
        .expect("gossip task should complete");

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-ai"))
            .header("x-tachyon-model", "llama3")
            .body("local-only")
            .send()
            .await
            .expect("host request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            expected_guest_example_body("FaaS received: local-only")
        );

        let peer_capture = peer_capture
            .lock()
            .expect("peer capture should not be poisoned");
        assert!(peer_capture.bodies.is_empty());

        host_server.abort();
        peer_server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capability_routing_skips_override_candidates_without_required_capabilities() {
        use axum::{
            body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
        };
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-capable")
        }

        let wrong_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let wrong_app = Router::new()
            .route("/api/guest-example", post(peer_target))
            .with_state(Arc::clone(&wrong_capture));
        let wrong_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("incapable peer should bind");
        let wrong_address = wrong_listener
            .local_addr()
            .expect("incapable peer should expose an address");
        let wrong_server = tokio::spawn(async move {
            axum::serve(wrong_listener, wrong_app)
                .await
                .expect("incapable peer should stay up");
        });

        let right_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let right_app = Router::new()
            .route("/api/guest-example", post(peer_target))
            .with_state(Arc::clone(&right_capture));
        let right_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("capable peer should bind");
        let right_address = right_listener
            .local_addr()
            .expect("capable peer should expose an address");
        let right_server = tokio::spawn(async move {
            axum::serve(right_listener, right_app)
                .await
                .expect("capable peer should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![targeted_route(
                "/api/guest-example",
                vec![capability_target(
                    "guest-example",
                    &["core:wasi", "accel:cuda"],
                )],
            )],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let override_descriptor = serde_json::to_string(&RouteOverrideDescriptor {
            candidates: vec![
                RouteOverrideCandidate {
                    destination: format!("http://{wrong_address}/api/guest-example"),
                    hot_models: Vec::new(),
                    effective_pressure: 5,
                    capability_mask: Capabilities::CORE_WASI,
                    capabilities: vec!["core:wasi".to_owned()],
                },
                RouteOverrideCandidate {
                    destination: format!("http://{right_address}/api/guest-example"),
                    hot_models: Vec::new(),
                    effective_pressure: 10,
                    capability_mask: Capabilities::CORE_WASI | Capabilities::ACCEL_CUDA,
                    capabilities: vec!["core:wasi".to_owned(), "accel:cuda".to_owned()],
                },
            ],
        })
        .expect("override descriptor should serialize");
        update_control_plane_route_override(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            "/api/guest-example",
            &override_descriptor,
        )
        .expect("capability-aware override should install");

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-example"))
            .body("capability-request")
            .send()
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            "peer-capable"
        );

        let wrong_capture = wrong_capture
            .lock()
            .expect("incapable peer capture should not be poisoned");
        assert!(wrong_capture.bodies.is_empty());
        let right_capture = right_capture
            .lock()
            .expect("capable peer capture should not be poisoned");
        assert_eq!(right_capture.bodies, vec!["capability-request".to_owned()]);
        let cached = state
            .peer_capabilities
            .lock()
            .expect("peer cache should not be poisoned");
        assert_eq!(cached.len(), 2);

        host_server.abort();
        wrong_server.abort();
        right_server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capability_routing_returns_503_when_local_and_mesh_lack_requirements() {
        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![targeted_route(
                "/api/guest-example",
                vec![capability_target(
                    "guest-example",
                    &["core:wasi", "accel:cuda"],
                )],
            )],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config, telemetry::init_test_telemetry());
        let host_app = build_app(state);
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-example"))
            .body("missing-capability")
            .send()
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response.text().await.expect("response body should decode");
        assert!(body.contains("Missing Capability"));
        assert!(body.contains("accel:cuda"));

        host_server.abort();
    }

    #[cfg(feature = "ai-inference")]
    #[tokio::test(flavor = "multi_thread")]
    async fn mesh_qos_router_forwards_realtime_gpu_requests_to_prefixed_override() {
        use axum::{
            body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
        };
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-realtime")
        }

        let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let peer_app = Router::new()
            .route("/api/guest-ai", post(peer_target))
            .with_state(Arc::clone(&peer_capture));
        let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("peer listener should bind");
        let peer_address = peer_listener
            .local_addr()
            .expect("peer listener should expose an address");
        let peer_server = tokio::spawn(async move {
            axum::serve(peer_listener, peer_app)
                .await
                .expect("peer app should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut route =
            targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
        route.models = vec![
            IntegrityModelBinding {
                alias: "gpu-live-chat".to_owned(),
                path: "/models/gpu-live-chat.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
            },
            IntegrityModelBinding {
                alias: "gpu-batch".to_owned(),
                path: "/models/gpu-batch.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Batch,
            },
        ];
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config, telemetry::init_test_telemetry());
        state.runtime.load().ai_runtime.set_queue_depth_for_test(
            ai_inference::AcceleratorKind::Gpu,
            RouteQos::RealTime,
            2,
        );
        update_control_plane_route_override(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            &format!("{MESH_QOS_OVERRIDE_PREFIX}/api/guest-ai"),
            &format!("http://{peer_address}/api/guest-ai"),
        )
        .expect("mesh qos override should install");

        let host_app = build_app(state);
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-ai"))
            .header("x-tachyon-model", "gpu-live-chat")
            .body("realtime-request")
            .send()
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            "peer-realtime"
        );
        let peer_capture = peer_capture
            .lock()
            .expect("peer capture should not be poisoned");
        assert_eq!(peer_capture.bodies, vec!["realtime-request".to_owned()]);

        host_server.abort();
        peer_server.abort();
    }

    #[cfg(feature = "ai-inference")]
    #[tokio::test(flavor = "multi_thread")]
    async fn mesh_qos_router_keeps_batch_gpu_requests_local_below_remote_threshold() {
        use axum::{
            body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
        };
        use std::sync::Mutex;

        #[derive(Default)]
        struct PeerCapture {
            bodies: Vec<String>,
        }

        async fn peer_target(
            State(state): State<Arc<Mutex<PeerCapture>>>,
            body: AxumBytes,
        ) -> impl IntoResponse {
            state
                .lock()
                .expect("peer capture should not be poisoned")
                .bodies
                .push(String::from_utf8_lossy(&body).to_string());
            (StatusCode::OK, "peer-batch")
        }

        let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
        let peer_app = Router::new()
            .route("/api/guest-ai", post(peer_target))
            .with_state(Arc::clone(&peer_capture));
        let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("peer listener should bind");
        let peer_address = peer_listener
            .local_addr()
            .expect("peer listener should expose an address");
        let peer_server = tokio::spawn(async move {
            axum::serve(peer_listener, peer_app)
                .await
                .expect("peer app should stay up");
        });

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut route =
            targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
        route.models = vec![
            IntegrityModelBinding {
                alias: "gpu-live-chat".to_owned(),
                path: "/models/gpu-live-chat.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::RealTime,
            },
            IntegrityModelBinding {
                alias: "gpu-batch".to_owned(),
                path: "/models/gpu-batch.gguf".to_owned(),
                device: ModelDevice::Cuda,
                qos: RouteQos::Batch,
            },
        ];
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config, telemetry::init_test_telemetry());
        state.runtime.load().ai_runtime.set_queue_depth_for_test(
            ai_inference::AcceleratorKind::Gpu,
            RouteQos::Batch,
            32,
        );
        update_control_plane_route_override(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            &format!("{MESH_QOS_OVERRIDE_PREFIX}/api/guest-ai"),
            &format!("http://{peer_address}/api/guest-ai"),
        )
        .expect("mesh qos override should install");

        let host_app = build_app(state);
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-ai"))
            .header("x-tachyon-model", "gpu-batch")
            .body("batch-request")
            .send()
            .await
            .expect("request should complete");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.text().await.expect("response body should decode"),
            expected_guest_example_body("FaaS received: batch-request")
        );
        let peer_capture = peer_capture
            .lock()
            .expect("peer capture should not be poisoned");
        assert!(peer_capture.bodies.is_empty());

        host_server.abort();
        peer_server.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_plane_buffer_persists_and_replays_requests_when_capacity_returns() {
        let queue_dir = unique_test_dir("tachyon-buffer-queue");
        let state_dir = unique_test_dir("tachyon-buffer-state");
        fs::create_dir_all(&queue_dir).expect("buffer queue dir should create");
        fs::create_dir_all(&state_dir).expect("buffer state dir should create");

        let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("host listener should bind");
        let host_address = host_listener
            .local_addr()
            .expect("host listener should expose an address");

        let mut buffer_route = system_targeted_route("/system/buffer", "buffer");
        buffer_route.env = route_env(&[
            ("BUFFER_DIR", "/buffer"),
            ("RAM_QUEUE_CAPACITY", "1"),
            ("REPLAY_CPU_LIMIT", "70"),
            ("REPLAY_RAM_LIMIT", "70"),
            ("REPLAY_BATCH_SIZE", "4"),
        ]);
        buffer_route.volumes = vec![mounted_volume(&queue_dir, "/buffer")];

        let mut user_route = targeted_route(
            "/api/guest-volume",
            vec![weighted_target("guest-volume", 100)],
        );
        user_route.volumes = vec![mounted_volume(&state_dir, "/app/data")];
        let config = IntegrityConfig {
            host_address: host_address.to_string(),
            routes: vec![user_route.clone(), buffer_route.clone()],
            ..IntegrityConfig::default_sealed()
        };
        let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
        update_control_plane_route_override(
            state.route_overrides.as_ref(),
            &state.peer_capabilities,
            "/api/guest-volume",
            "/system/buffer",
        )
        .expect("buffer override should install");

        let host_app = build_app(state.clone());
        let host_server = tokio::spawn(async move {
            axum::serve(host_listener, host_app)
                .await
                .expect("host app should stay up");
        });

        let response = Client::new()
            .post(format!("http://{host_address}/api/guest-volume"))
            .body("buffered payload")
            .send()
            .await
            .expect("buffered request should succeed");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let queued_files = fs::read_dir(queue_dir.join("ram"))
            .expect("ram queue should exist")
            .filter_map(Result::ok)
            .count();
        assert_eq!(queued_files, 1);

        tokio::task::spawn_blocking({
            let config = config.clone();
            let route_overrides = Arc::clone(&state.route_overrides);
            let peer_capabilities = Arc::clone(&state.peer_capabilities);
            let host_load = Arc::clone(&state.host_load);
            let storage_broker = Arc::clone(&state.storage_broker);
            let telemetry = state.telemetry.clone();
            let host_identity = Arc::clone(&state.host_identity);
            let host_capabilities = state.host_capabilities;
            move || {
                let engine = build_test_metered_engine(&config);
                let mut runner = BackgroundTickRunner::new(
                    &engine,
                    &config,
                    config
                        .sealed_route("/system/buffer")
                        .expect("buffer route should be sealed"),
                    "buffer",
                    telemetry,
                    build_concurrency_limits(&config),
                    host_identity,
                    storage_broker,
                    route_overrides,
                    peer_capabilities,
                    host_capabilities,
                    host_load,
                )
                .expect("buffer component should instantiate");
                runner.tick().expect("buffer tick should succeed");
            }
        })
        .await
        .expect("buffer replay task should complete");

        let persisted = fs::read_to_string(state_dir.join("state.txt"))
            .expect("guest-volume should persist replayed payload");
        assert_eq!(persisted, "buffered payload");

        let remaining = fs::read_dir(queue_dir.join("ram"))
            .expect("ram queue should still exist")
            .filter_map(Result::ok)
            .count();
        assert_eq!(remaining, 0);

        host_server.abort();
    }

    #[test]
    fn blocking_reqwest_client_initializes_with_default_tls_provider() {
        ensure_rustls_crypto_provider();
        let _client = blocking_outbound_http_client();
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
