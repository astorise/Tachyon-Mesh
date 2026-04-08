use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri},
    middleware::from_fn,
    response::{IntoResponse, Response},
    Extension, Router,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand::Rng;
use reqwest::Client;
use semver::{Version, VersionReq};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt, fs,
    io::Write,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Once,
    },
    time::{Duration, Instant, SystemTime},
};
use telemetry::{TelemetryEvent, TelemetryHandle, TelemetrySnapshot};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, TryAcquireError};
use uuid::Uuid;
use wasmtime::{
    component::{Component, Linker as ComponentLinker},
    Config, Engine, Instance, Linker as ModuleLinker, Module, PoolingAllocationConfig,
    ResourceLimiter, Store, Trap, TypedFunc,
};
use wasmtime_wasi::{
    p1::{self, WasiP1Ctx},
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
    DirPerms, FilePerms, I32Exit, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView,
};
#[cfg(feature = "ai-inference")]
use wasmtime_wasi_nn::{backend, witx::WasiNnCtx, Backend as WasiNnBackend, InMemoryRegistry};

#[cfg(feature = "rate-limit")]
mod rate_limit;
mod telemetry;

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

mod background_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "background-system-faas",
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
const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");
const INTEGRITY_MANIFEST_PATH_ENV: &str = "TACHYON_INTEGRITY_MANIFEST";
const DEFAULT_HOP_LIMIT: u32 = 10;
const HOP_LIMIT_HEADER: &str = "x-tachyon-hop-limit";
const COHORT_HEADER: &str = "x-cohort";
const TACHYON_COHORT_HEADER: &str = "x-tachyon-cohort";
const SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD: usize = 32;
const DEFAULT_ROUTE_MAX_CONCURRENCY: u32 = 100;
const DEFAULT_ROUTE_VERSION: &str = "0.0.0";
const AUTOSCALING_TICK_INTERVAL: Duration = Duration::from_secs(5);
const VOLUME_GC_TICK_INTERVAL: Duration = Duration::from_secs(60);
const KUBERNETES_SERVICE_BASE_URL: &str = "https://kubernetes.default.svc";
const MOCK_K8S_URL_ENV: &str = "TACHYON_MOCK_K8S_URL";
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

fn is_default_route_version(version: &String) -> bool {
    version == DEFAULT_ROUTE_VERSION
}

fn default_volume_type() -> VolumeType {
    VolumeType::Host
}

fn is_default_volume_type(volume_type: &VolumeType) -> bool {
    *volume_type == VolumeType::Host
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<ArcSwap<RuntimeState>>,
    http_client: Client,
    secrets_vault: SecretsVault,
    storage_broker: Arc<StorageBrokerManager>,
    volume_manager: Arc<VolumeManager>,
    telemetry: TelemetryHandle,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    manifest_path: PathBuf,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    background_workers: Arc<BackgroundWorkerManager>,
}

#[derive(Clone)]
struct RuntimeState {
    engine: Engine,
    config: IntegrityConfig,
    route_registry: Arc<RouteRegistry>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HopLimit(u32);

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
    route_path: String,
    volumes: Vec<IntegrityVolume>,
    storage_broker: Arc<StorageBrokerManager>,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
    outbound_http_client: reqwest::blocking::Client,
}

#[derive(Clone)]
struct GuestTelemetryContext {
    handle: TelemetryHandle,
    trace_id: String,
}

struct GuestExecutionContext {
    config: IntegrityConfig,
    runtime_telemetry: TelemetryHandle,
    secret_access: SecretAccess,
    storage_broker: Arc<StorageBrokerManager>,
    telemetry: Option<GuestTelemetryContext>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    propagated_headers: Vec<PropagatedHeader>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestRequest {
    method: String,
    uri: String,
    body: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuestHttpResponse {
    status: StatusCode,
    body: Bytes,
}

#[derive(Debug, PartialEq, Eq)]
enum GuestExecutionOutput {
    Http(GuestHttpResponse),
    LegacyStdout(Bytes),
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
struct RouteTarget {
    module: String,
    #[serde(default)]
    weight: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    match_header: Option<HeaderMatch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PropagatedHeader {
    name: String,
    value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceLimitKind {
    Fuel,
    Memory,
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
}

#[derive(Clone, Default)]
struct StorageBrokerManager {
    queues: Arc<Mutex<HashMap<PathBuf, Arc<StorageVolumeQueue>>>>,
}

struct StorageVolumeQueue {
    volume_root: PathBuf,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    allowed_secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    targets: Vec<RouteTarget>,
    #[serde(default)]
    min_instances: u32,
    #[serde(default = "default_max_concurrency")]
    max_concurrency: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<IntegrityVolume>,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
struct IntegrityConfig {
    host_address: String,
    max_stdout_bytes: usize,
    guest_fuel_budget: u64,
    guest_memory_limit_bytes: usize,
    resource_limit_response: String,
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
    let telemetry = telemetry::init_telemetry();
    let runtime = build_runtime_state(verify_integrity()?)?;
    let storage_broker = Arc::new(StorageBrokerManager::default());
    let background_workers = Arc::new(BackgroundWorkerManager::default());
    background_workers.start_for_runtime(&runtime, telemetry.clone(), Arc::clone(&storage_broker));

    let state = AppState {
        runtime: Arc::new(ArcSwap::from_pointee(runtime.clone())),
        http_client: Client::new(),
        secrets_vault: SecretsVault::load(),
        storage_broker,
        volume_manager: Arc::new(VolumeManager::default()),
        telemetry,
        manifest_path: integrity_manifest_path(),
        background_workers: Arc::clone(&background_workers),
    };
    spawn_reload_watcher(state.clone());
    spawn_volume_gc_sweeper(state.clone());
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(&runtime.config.host_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind HTTP listener on {}",
                runtime.config.host_address.as_str()
            )
        })?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("axum server exited unexpectedly")?;

    background_workers.stop_all().await;
    Ok(())
}

impl BackgroundWorkerManager {
    fn start_for_runtime(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
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
                &runtime.engine,
                &runtime.config,
                route,
                &function_name,
                telemetry.clone(),
                Arc::clone(&runtime.concurrency_limits),
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
            let worker_engine = runtime.engine.clone();
            let worker_config = runtime.config.clone();
            let worker_telemetry = telemetry.clone();
            let worker_limits = Arc::clone(&runtime.concurrency_limits);
            let worker_storage_broker = Arc::clone(&storage_broker);
            let worker_stop = Arc::clone(&stop_requested);
            let join_handle = tokio::task::spawn_blocking(move || {
                run_background_tick_loop(
                    worker_engine,
                    worker_config,
                    worker_telemetry,
                    worker_limits,
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
        storage_broker: Arc<StorageBrokerManager>,
    ) {
        self.stop_all().await;
        self.start_for_runtime(runtime, telemetry, storage_broker);
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

fn run_background_tick_loop(
    engine: Engine,
    config: IntegrityConfig,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
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
    fn enqueue_write_for_route(
        &self,
        route: &IntegrityRoute,
        path: &str,
        mode: StorageWriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let resolved = resolve_storage_write_target(route, path)?;
        let queue = self.queue_for_volume(&resolved.volume_root);
        queue.enqueue(StorageBrokerOperation::Write(StorageBrokerWriteRequest {
            route_path: route.path.clone(),
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
                .or_insert_with(|| StorageVolumeQueue::new(key)),
        )
    }

    #[cfg(test)]
    fn wait_for_volume_idle(&self, volume_root: &Path, timeout: Duration) -> bool {
        self.queue_for_volume(volume_root).wait_for_idle(timeout)
    }
}

impl StorageVolumeQueue {
    fn new(volume_root: PathBuf) -> Arc<Self> {
        let (sender, receiver) = std::sync::mpsc::channel::<StorageBrokerOperation>();
        let queue = Arc::new(Self {
            volume_root,
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
                    let result = process_storage_snapshot_request(&request)
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
                    let result = process_storage_restore_request(&request)
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
        .filter(|volume| !volume.readonly)
        .filter(|volume| guest_path_matches_volume(&normalized_path, &volume.guest_path))
        .max_by_key(|volume| volume.guest_path.len())
        .ok_or_else(|| {
            format!(
                "route `{}` cannot broker writes to `{normalized_path}` because no writable mounted volume matches that path",
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

fn process_storage_snapshot_request(request: &StorageBrokerSnapshotRequest) -> Result<()> {
    copy_directory_tree(&request.source_path, &request.snapshot_path)?;
    remove_path_if_exists(&request.source_path)?;
    Ok(())
}

fn process_storage_restore_request(request: &StorageBrokerRestoreRequest) -> Result<()> {
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

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
async fn reload_runtime_from_disk(state: &AppState) -> Result<()> {
    let manifest_path = state.manifest_path.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        let config = load_integrity_config_from_manifest_path(&manifest_path)?;
        build_runtime_state(config)
    })
    .await
    .context("hot reload task failed")??;

    state
        .background_workers
        .replace_with(
            &runtime,
            state.telemetry.clone(),
            Arc::clone(&state.storage_broker),
        )
        .await;
    state.runtime.store(Arc::new(runtime));
    tracing::info!(
        manifest = %state.manifest_path.display(),
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
        engine: build_engine(&config)?,
        route_registry: Arc::new(RouteRegistry::build(&config)?),
        concurrency_limits: build_concurrency_limits(&config),
        config,
    })
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

    #[cfg(feature = "rate-limit")]
    let app = app.layer(axum::middleware::from_fn_with_state(
        rate_limit::new_rate_limiter(),
        rate_limit::rate_limit_middleware,
    ));

    app.with_state(state)
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

async fn faas_handler(
    State(state): State<AppState>,
    Extension(hop_limit): Extension<HopLimit>,
    headers: HeaderMap,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    let _active_request = telemetry::begin_request(&state.telemetry);
    let runtime = state.runtime.load_full();
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

    let response: Response = match runtime.config.sealed_route(&normalized_path).cloned() {
        None => (
            StatusCode::NOT_FOUND,
            format!("route `{normalized_path}` is not sealed in `integrity.lock`"),
        )
            .into_response(),
        Some(route) => match execute_route_with_middleware(
            &state, &runtime, &route, &headers, &method, &uri, &body, hop_limit, &trace_id,
        )
        .await
        {
            Ok(response) => (response.status, response.body).into_response(),
            Err((status, message)) => (status, message).into_response(),
        },
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

async fn execute_route_with_middleware(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    hop_limit: HopLimit,
    trace_id: &str,
) -> std::result::Result<GuestHttpResponse, (StatusCode, String)> {
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
            hop_limit,
            trace_id,
        )
        .await?;
        if middleware_response.status != StatusCode::OK {
            return Ok(middleware_response);
        }
    }

    execute_route_request(
        state, runtime, route, headers, method, uri, body, hop_limit, trace_id,
    )
    .await
}

async fn execute_route_request(
    state: &AppState,
    runtime: &Arc<RuntimeState>,
    route: &IntegrityRoute,
    headers: &HeaderMap,
    method: &Method,
    uri: &Uri,
    body: &Bytes,
    hop_limit: HopLimit,
    trace_id: &str,
) -> std::result::Result<GuestHttpResponse, (StatusCode, String)> {
    if route.role == RouteRole::System && should_shed_system_route(&state.telemetry) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("system route `{}` shed under load", route.path),
        ));
    }

    let _volume_leases = state
        .volume_manager
        .acquire_route_volumes(route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error))?;

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
    let _permit = match acquire_route_permit(semaphore).await {
        Ok(permit) => permit,
        Err(RoutePermitError::Closed) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("route `{}` is currently unavailable", route.path),
            ));
        }
        Err(RoutePermitError::TimedOut) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("route `{}` is saturated", route.path),
            ));
        }
    };

    let selected_module = select_route_module(route, headers)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let propagated_headers = extract_propagated_headers(headers);
    let engine = runtime.engine.clone();
    let request_config = runtime.config.clone();
    let response_config = runtime.config.clone();
    let route_registry = Arc::clone(&runtime.route_registry);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let storage_broker = Arc::clone(&state.storage_broker);
    let telemetry_context = GuestTelemetryContext {
        handle: state.telemetry.clone(),
        trace_id: trace_id.to_owned(),
    };
    let runtime_telemetry = state.telemetry.clone();
    let secret_access = SecretAccess::from_route(route, &state.secrets_vault);
    let task_route = route.clone();
    let task_function_name = selected_module.clone();
    let task_propagated_headers = propagated_headers.clone();
    let guest_request = GuestRequest {
        method: method.to_string(),
        uri: uri.to_string(),
        body: body.clone(),
    };
    let result = tokio::task::spawn_blocking(move || {
        execute_guest(
            &engine,
            &task_function_name,
            guest_request,
            &task_route,
            GuestExecutionContext {
                config: request_config,
                runtime_telemetry,
                secret_access,
                storage_broker,
                telemetry: Some(telemetry_context),
                concurrency_limits,
                propagated_headers: task_propagated_headers,
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

    let response = match result {
        Ok(output) => match output {
            GuestExecutionOutput::Http(response) => response,
            GuestExecutionOutput::LegacyStdout(stdout) => GuestHttpResponse {
                status: StatusCode::OK,
                body: stdout,
            },
        },
        Err(error) => {
            error.log_if_needed(&selected_module);
            let (status, message) = error.into_response(&response_config);
            return Err((status, message));
        }
    };

    resolve_mesh_response(
        &state.http_client,
        &response_config,
        &route_registry,
        route,
        hop_limit,
        &propagated_headers,
        response,
    )
    .await
    .map_err(|error| (StatusCode::BAD_GATEWAY, error))
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

async fn resolve_mesh_response(
    http_client: &Client,
    config: &IntegrityConfig,
    route_registry: &RouteRegistry,
    caller_route: &IntegrityRoute,
    hop_limit: HopLimit,
    propagated_headers: &[PropagatedHeader],
    response: GuestHttpResponse,
) -> std::result::Result<GuestHttpResponse, String> {
    let Some(target) = extract_mesh_fetch_url(&response.body) else {
        return Ok(response);
    };
    let url = resolve_mesh_fetch_target(config, route_registry, caller_route, target)?;

    let mut request = http_client
        .get(&url)
        .header(HOP_LIMIT_HEADER, hop_limit.decremented().to_string());
    for header in propagated_headers {
        request = request.header(&header.name, &header.value);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("mesh fetch to `{url}` failed: {error}"))?;

    let status = response.status();
    let body = response.bytes().await.map_err(|error| {
        format!("failed to read mesh fetch response body from `{url}`: {error}")
    })?;

    if status == StatusCode::LOOP_DETECTED || status.is_success() {
        Ok(GuestHttpResponse { status, body })
    } else {
        Err(format!(
            "mesh fetch to `{url}` returned an error status: {status}"
        ))
    }
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
    select_route_module_with_roll(route, headers, None)
}

fn select_route_module_with_roll(
    route: &IntegrityRoute,
    headers: &HeaderMap,
    random_roll: Option<u64>,
) -> std::result::Result<String, String> {
    for target in &route.targets {
        if target
            .match_header
            .as_ref()
            .is_some_and(|matcher| request_header_matches(headers, matcher))
        {
            return Ok(target.module.clone());
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
                return Ok(target.module.clone());
            }
        }
    }

    resolve_function_name(&route.path).ok_or_else(|| {
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

fn client_connect_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
        IpAddr::V4(ip) => ip.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("[{}]", Ipv6Addr::LOCALHOST),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
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
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
    #[cfg(not(feature = "ai-inference"))]
    if requires_ai_inference_feature(function_name) {
        return Err(ExecutionError::Internal(format!(
            "guest `{function_name}` requires `core-host` to be built with `--features ai-inference`"
        )));
    }

    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;

    if let Ok(component) = Component::from_file(engine, &module_path) {
        return match route.role {
            RouteRole::User => execute_component_guest(
                engine,
                request,
                route,
                &module_path,
                &component,
                &execution,
            ),
            RouteRole::System => execute_system_component_guest(
                engine,
                request,
                route,
                &module_path,
                &component,
                &execution,
            ),
        };
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
        route,
        &module_path,
        module,
        &execution,
    )
}

fn execute_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
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
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(execution.config.guest_fuel_budget)
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
    record_wasm_start(execution.telemetry.as_ref());
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            body: request.body.to_vec(),
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
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

fn execute_system_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
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
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(execution.config.guest_fuel_budget)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;

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
            body: request.body.to_vec(),
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
    let response = response.map_err(|error| {
        guest_execution_error(error, "system guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "system guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutput::Http(GuestHttpResponse {
        status,
        body: Bytes::from(response.body),
    }))
}

impl BackgroundTickRunner {
    fn new(
        engine: &Engine,
        config: &IntegrityConfig,
        route: &IntegrityRoute,
        function_name: &str,
        telemetry: TelemetryHandle,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
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
                config.guest_memory_limit_bytes,
                telemetry,
                SecretAccess::default(),
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
) -> std::result::Result<GuestExecutionOutput, ExecutionError> {
    let linker = build_linker(engine)?;
    let stdout = MemoryOutputPipe::new(execution.config.max_stdout_bytes);
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

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(wasi, execution.config.guest_memory_limit_bytes),
    );
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(execution.config.guest_fuel_budget)
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

    record_wasm_start(execution.telemetry.as_ref());
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(execution.telemetry.as_ref());
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

fn build_engine(integrity_config: &IntegrityConfig) -> Result<Engine> {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.wasm_component_model(true);
    config.allocation_strategy(build_pooling_config(integrity_config)?);

    Engine::new(&config)
        .map_err(|error| anyhow!("failed to create Wasmtime engine with pooling enabled: {error}"))
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
                    Arc::new(RouteExecutionControl::new(route.max_concurrency)),
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
    fn new(max_concurrency: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(
                usize::try_from(max_concurrency)
                    .expect("route max_concurrency should fit in usize"),
            )),
            pending_waiters: AtomicUsize::new(0),
        }
    }

    fn pending_queue_size(&self) -> u32 {
        self.pending_waiters
            .load(Ordering::Relaxed)
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

    config.routes = normalize_config_routes(config.routes)?;
    RouteRegistry::build(&config)?;
    Ok(config)
}

fn normalize_config_routes(routes: Vec<IntegrityRoute>) -> Result<Vec<IntegrityRoute>> {
    if routes.is_empty() {
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
        allowed_secrets: normalize_allowed_secrets(route.allowed_secrets)?,
        targets: normalize_route_targets(route.targets)?,
        min_instances: route.min_instances,
        max_concurrency: route.max_concurrency,
        volumes: normalize_route_volumes(route.volumes, route.role, &normalized)?,
    })
}

fn normalize_route_targets(targets: Vec<RouteTarget>) -> Result<Vec<RouteTarget>> {
    targets
        .into_iter()
        .map(normalize_route_target)
        .collect::<Result<Vec<_>>>()
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
            #[cfg(feature = "ai-inference")]
            wasi_nn: build_wasi_nn_ctx(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
        }
    }
}

#[cfg(feature = "ai-inference")]
fn build_wasi_nn_ctx() -> WasiNnCtx {
    let registry = InMemoryRegistry::new();
    let backends = [WasiNnBackend::from(backend::onnx::OnnxBackend::default())];
    WasiNnCtx::new(backends, registry.into())
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
            return Err(SecretAccessErrorKind::VaultDisabled);
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
    fn new(
        route: &IntegrityRoute,
        max_memory_bytes: usize,
        telemetry: TelemetryHandle,
        secrets: SecretAccess,
        storage_broker: Arc<StorageBrokerManager>,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
        propagated_headers: Vec<PropagatedHeader>,
    ) -> std::result::Result<Self, ExecutionError> {
        Ok(Self {
            ctx: build_component_wasi_ctx(route)?,
            table: ResourceTable::new(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
            secrets,
            route_path: route.path.clone(),
            volumes: route.volumes.clone(),
            storage_broker,
            telemetry,
            concurrency_limits,
            propagated_headers,
            outbound_http_client: reqwest::blocking::Client::new(),
        })
    }

    fn pending_queue_size(&self, route_path: &str) -> u32 {
        self.concurrency_limits
            .get(&normalize_route_path(route_path))
            .map(|control| control.pending_queue_size())
            .unwrap_or_default()
    }
}

fn build_component_wasi_ctx(
    route: &IntegrityRoute,
) -> std::result::Result<WasiCtx, ExecutionError> {
    // Intentionally do not inherit the host environment. Secrets stay in host memory
    // and are only reachable through the typed vault import.
    let mut wasi = WasiCtxBuilder::new();
    preopen_route_volumes(&mut wasi, route)?;
    Ok(wasi.build())
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
        let route = IntegrityRoute {
            path: self.route_path.clone(),
            role: RouteRole::System,
            name: String::new(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
            min_instances: 0,
            max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
            volumes: self.volumes.clone(),
        };

        self.storage_broker
            .enqueue_write_for_route(&route, &path, mode, body)
    }

    fn snapshot_volume(
        &mut self,
        volume_id: String,
        source_path: String,
        snapshot_path: String,
    ) -> std::result::Result<(), String> {
        let source_path = parse_storage_broker_host_path(&source_path, "source_path")?;
        let snapshot_path = parse_storage_broker_host_path(&snapshot_path, "snapshot_path")?;
        let _ = self.storage_broker.enqueue_snapshot(
            volume_id,
            &source_path,
            &source_path,
            &snapshot_path,
        )?;
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
        let _ = self.storage_broker.enqueue_restore(
            volume_id,
            &destination_path,
            &snapshot_path,
            &destination_path,
        )?;
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
        body: Vec<u8>,
    ) -> std::result::Result<u16, String> {
        let method = reqwest::Method::from_bytes(method.trim().as_bytes())
            .map_err(|error| format!("invalid outbound HTTP method `{method}`: {error}"))?;
        let url = rewrite_outbound_http_url(&url);

        tracing::info!(
            method = %method,
            url = %url,
            bytes = body.len(),
            "autoscaling guest sending outbound HTTP request"
        );

        let mut request = self
            .outbound_http_client
            .request(method, &url)
            .header("content-type", "application/merge-patch+json");
        for header in &self.propagated_headers {
            request = request.header(&header.name, &header.value);
        }
        let response = request
            .body(body)
            .send()
            .map_err(|error| format!("failed to send outbound HTTP request to `{url}`: {error}"))?;

        Ok(response.status().as_u16())
    }
}

fn rewrite_outbound_http_url(url: &str) -> String {
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

impl IntegrityConfig {
    #[cfg(test)]
    fn default_sealed() -> Self {
        Self {
            host_address: DEFAULT_HOST_ADDRESS.to_owned(),
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
            allowed_secrets: allowed_secrets
                .iter()
                .map(|secret| (*secret).to_owned())
                .collect(),
            targets: Vec::new(),
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
    use http_body_util::BodyExt;
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    use tower::util::ServiceExt;

    type CapturedForwardedHeaders = Arc<std::sync::Mutex<Vec<(String, String, String)>>>;

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

    fn build_test_engine(config: &IntegrityConfig) -> Engine {
        build_engine(config).expect("engine should be created")
    }

    fn build_test_runtime(config: IntegrityConfig) -> RuntimeState {
        build_runtime_state(config).expect("runtime state should build")
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
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            guest_fuel_budget: DEFAULT_GUEST_FUEL_BUDGET,
            guest_memory_limit_bytes: DEFAULT_GUEST_MEMORY_LIMIT_BYTES,
            resource_limit_response: DEFAULT_RESOURCE_LIMIT_RESPONSE.to_owned(),
            routes,
        }
    }

    fn build_test_state(config: IntegrityConfig, telemetry: TelemetryHandle) -> AppState {
        build_test_state_with_manifest(config, telemetry, PathBuf::from("integrity.lock"))
    }

    fn build_test_state_with_manifest(
        config: IntegrityConfig,
        telemetry: TelemetryHandle,
        manifest_path: PathBuf,
    ) -> AppState {
        AppState {
            runtime: Arc::new(ArcSwap::from_pointee(build_test_runtime(config))),
            http_client: Client::new(),
            secrets_vault: SecretsVault::load(),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            volume_manager: Arc::new(VolumeManager::default()),
            telemetry,
            manifest_path,
            background_workers: Arc::new(BackgroundWorkerManager::default()),
        }
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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

    fn storage_broker_test_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: "/system/storage-broker".to_owned(),
            role: RouteRole::System,
            name: "storage-broker".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            allowed_secrets: Vec::new(),
            targets: vec![RouteTarget {
                module: "system-faas-storage-broker".to_owned(),
                weight: 100,
                match_header: None,
            }],
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

    fn hibernating_ram_route(host_path: &std::path::Path) -> IntegrityRoute {
        IntegrityRoute {
            path: "/api/guest-volume".to_owned(),
            role: RouteRole::User,
            name: "guest-volume".to_owned(),
            version: default_route_version(),
            dependencies: BTreeMap::new(),
            requires_credentials: Vec::new(),
            middleware: None,
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
            match_header: None,
        }
    }

    fn header_target(module: &str, header_name: &str, header_value: &str) -> RouteTarget {
        RouteTarget {
            module: module.to_owned(),
            weight: 0,
            match_header: Some(HeaderMatch {
                name: header_name.to_owned(),
                value: header_value.to_owned(),
            }),
        }
    }

    fn unique_test_dir(prefix: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
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

    #[test]
    fn execute_guest_returns_component_response_payload() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = config
            .sealed_route("/api/guest-example")
            .expect("sealed route should exist")
            .clone();
        let response = execute_guest(
            &engine,
            "guest-example",
            GuestRequest {
                method: "POST".to_owned(),
                uri: "/api/guest-example".to_owned(),
                body: Bytes::from("Hello Lean FaaS!"),
            },
            &route,
            GuestExecutionContext {
                secret_access: SecretAccess::from_route(&route, &SecretsVault::load()),
                config,
                runtime_telemetry: telemetry::init_test_telemetry(),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
            },
        )
        .expect("guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutput::Http(GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from(expected_guest_example_body(
                    "FaaS received: Hello Lean FaaS!"
                )),
            })
        );
    }

    #[test]
    fn execute_guest_falls_back_to_legacy_stdout_for_non_component_module() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = IntegrityRoute::user("/api/guest-call-legacy");
        let response = execute_guest(
            &engine,
            "guest-call-legacy",
            GuestRequest {
                method: "GET".to_owned(),
                uri: "/api/guest-call-legacy".to_owned(),
                body: Bytes::new(),
            },
            &route,
            GuestExecutionContext {
                config,
                runtime_telemetry: telemetry::init_test_telemetry(),
                secret_access: SecretAccess::default(),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
            },
        )
        .expect("legacy guest execution should succeed");

        assert_eq!(
            response,
            GuestExecutionOutput::LegacyStdout(Bytes::from(
                "MESH_FETCH:http://legacy-service:8081/ping\n"
            ))
        );
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

        let save_response = execute_guest(
            &engine,
            "guest-volume",
            GuestRequest {
                method: "POST".to_owned(),
                uri: "/api/guest-volume".to_owned(),
                body: Bytes::from("Hello Stateful World"),
            },
            &route,
            GuestExecutionContext {
                config: config.clone(),
                runtime_telemetry: telemetry::init_test_telemetry(),
                secret_access: SecretAccess::default(),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
            },
        )
        .expect("volume guest should write successfully");

        assert_eq!(
            save_response,
            GuestExecutionOutput::Http(GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from("Saved"),
            })
        );

        let read_response = execute_guest(
            &engine,
            "guest-volume",
            GuestRequest {
                method: "GET".to_owned(),
                uri: "/api/guest-volume".to_owned(),
                body: Bytes::new(),
            },
            &route,
            GuestExecutionContext {
                config: config.clone(),
                runtime_telemetry: telemetry::init_test_telemetry(),
                secret_access: SecretAccess::default(),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&config),
                propagated_headers: Vec::new(),
            },
        )
        .expect("volume guest should read successfully");

        assert_eq!(
            read_response,
            GuestExecutionOutput::Http(GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from("Hello Stateful World"),
            })
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
            route_registry: Arc::new(
                RouteRegistry::build(&config).expect("route registry should build"),
            ),
            concurrency_limits: Arc::new(HashMap::from([(
                DEFAULT_ROUTE.to_owned(),
                Arc::new(RouteExecutionControl::new(0)),
            )])),
            config,
        };
        let app = build_app(AppState {
            runtime: Arc::new(ArcSwap::from_pointee(runtime)),
            http_client: Client::new(),
            secrets_vault: SecretsVault::load(),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            volume_manager: Arc::new(VolumeManager::default()),
            telemetry: telemetry::init_test_telemetry(),
            manifest_path: PathBuf::from("integrity.lock"),
            background_workers: Arc::new(BackgroundWorkerManager::default()),
        });

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

        assert!(String::from_utf8_lossy(&body).contains("saturated"));
    }

    #[test]
    fn system_guest_requires_system_route_role() {
        let config = IntegrityConfig::default_sealed();
        let engine = build_test_engine(&config);
        let route = IntegrityRoute::user("/metrics");
        let error = execute_guest(
            &engine,
            "metrics",
            GuestRequest {
                method: "GET".to_owned(),
                uri: "/metrics".to_owned(),
                body: Bytes::new(),
            },
            &route,
            GuestExecutionContext {
                config,
                runtime_telemetry: telemetry::init_test_telemetry(),
                secret_access: SecretAccess::default(),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
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
            let engine = build_test_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/k8s-scaler")
                    .expect("background route should be sealed"),
                "k8s-scaler",
                telemetry::init_test_telemetry(),
                concurrency_limits,
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
            select_route_module_with_roll(&route, &headers, Some(42))
                .expect("header-target route should resolve"),
            "guest-loop"
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
            select_route_module_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("weighted route should resolve"),
            "guest-example"
        );
        assert_eq!(
            select_route_module_with_roll(&route, &HeaderMap::new(), Some(89))
                .expect("weighted route should resolve"),
            "guest-example"
        );
        assert_eq!(
            select_route_module_with_roll(&route, &HeaderMap::new(), Some(90))
                .expect("weighted route should resolve"),
            "guest-loop"
        );
    }

    #[test]
    fn select_route_module_falls_back_to_path_module_when_targets_are_header_only() {
        let route = targeted_route(
            "/api/guest-example",
            vec![header_target("guest-loop", COHORT_HEADER, "beta")],
        );

        assert_eq!(
            select_route_module_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("route should fall back to the path module"),
            "guest-example"
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
        let propagated_headers = extract_propagated_headers(&inbound_headers);
        let config = IntegrityConfig::default_sealed();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            HopLimit(DEFAULT_HOP_LIMIT),
            &propagated_headers,
            GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from(format!("MESH_FETCH:http://{address}/ping")),
            },
        )
        .await
        .expect("mesh fetch should succeed");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("ok"));
        assert_eq!(
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .as_slice(),
            &[(
                (DEFAULT_HOP_LIMIT - 1).to_string(),
                "beta".to_owned(),
                "beta".to_owned()
            )]
        );
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
            GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from("middleware allowed"),
            },
        );
        responses.insert(
            "/api/protected-allow".to_owned(),
            GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from(expected_guest_example_body(
                    "FaaS received an empty payload",
                )),
            },
        );
        responses.insert(
            "/api/deny-middleware".to_owned(),
            GuestHttpResponse {
                status: StatusCode::FORBIDDEN,
                body: Bytes::from("forbidden"),
            },
        );
        responses.insert(
            "/api/protected-deny".to_owned(),
            GuestHttpResponse {
                status: StatusCode::OK,
                body: Bytes::from("main route should not execute"),
            },
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
        let snapshot_path = snapshot_path_for_volume(&volume_dir);

        for _ in 0..50 {
            if managed.lifecycle() == ManagedVolumeLifecycle::OnDisk {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        assert_eq!(managed.lifecycle(), ManagedVolumeLifecycle::OnDisk);
        assert!(
            snapshot_path.join("state.txt").exists(),
            "snapshot should contain the persisted state file"
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
        let _ = fs::remove_dir_all(snapshot_path);
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
            allowed_secrets: Vec::new(),
            targets: Vec::new(),
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
