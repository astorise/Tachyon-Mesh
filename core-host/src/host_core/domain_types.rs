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
    /// Optional target module or internal URL that receives a fire-and-forget copy
    /// of primary traffic through `system-faas-shadow-proxy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shadow_target: Option<String>,
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
            shadow_target: None,
            adapter_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AdmissionStrategy {
    #[default]
    FailFast,
    MeshRetry,
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
    /// Request count permitted across the entire mesh within `window_seconds`.
    threshold: u32,
    #[serde(default = "default_dist_rate_limit_window")]
    window_seconds: u32,
    #[serde(default, skip_serializing_if = "is_default_dist_rate_limit_scope")]
    scope: DistributedRateLimitScope,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DistributedRateLimitScope {
    #[default]
    Ip,
    Tenant,
    Token,
}

fn is_default_dist_rate_limit_scope(scope: &DistributedRateLimitScope) -> bool {
    *scope == DistributedRateLimitScope::Ip
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
    #[arg(long, value_enum, default_value_t = AccelerationMode::Userspace)]
    accel: AccelerationMode,
    #[command(subcommand)]
    command: Option<HostCommand>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum AccelerationMode {
    #[default]
    Userspace,
    Ebpf,
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
