use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResourceLimitKind {
    Fuel,
    Memory,
    Stdout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoutePermitError {
    Closed,
    TimedOut,
}

#[cfg_attr(not(feature = "secrets-vault"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SecretAccessErrorKind {
    NotFound,
    PermissionDenied,
    #[cfg(not(feature = "secrets-vault"))]
    VaultDisabled,
}

#[derive(Debug)]
pub(crate) struct ResourceLimitTrap {
    pub(crate) kind: ResourceLimitKind,
}

#[derive(Debug)]
pub(crate) struct GuestModuleNotFound {
    pub(crate) function_name: String,
    pub(crate) candidate_paths: String,
}

pub(crate) struct RouteExecutionControl {
    pub(crate) semaphore: Arc<Semaphore>,
    pub(crate) pending_waiters: AtomicUsize,
    pub(crate) active_requests: AtomicUsize,
    pub(crate) draining: AtomicBool,
    pub(crate) draining_since: Mutex<Option<Instant>>,
    pub(crate) min_instances: u32,
    pub(crate) max_concurrency: u32,
    pub(crate) prewarmed_instances: AtomicUsize,
}

#[derive(Clone)]
pub(crate) struct StorageBrokerManager {
    pub(crate) core_store: Arc<store::CoreStore>,
    pub(crate) queues: Arc<Mutex<HashMap<PathBuf, Arc<StorageVolumeQueue>>>>,
}

pub(crate) struct StorageVolumeQueue {
    pub(crate) volume_root: PathBuf,
    pub(crate) core_store: Arc<store::CoreStore>,
    pub(crate) sender: std::sync::mpsc::Sender<StorageBrokerOperation>,
    pub(crate) state: Mutex<StorageVolumeQueueState>,
    pub(crate) idle: Condvar,
}

#[derive(Default)]
pub(crate) struct StorageVolumeQueueState {
    pub(crate) pending: usize,
}

#[derive(Debug)]
pub(crate) enum StorageBrokerOperation {
    Write(StorageBrokerWriteRequest),
    Snapshot(StorageBrokerSnapshotRequest),
    Restore(StorageBrokerRestoreRequest),
}

#[derive(Clone, Debug)]
pub(crate) struct StorageBrokerWriteRequest {
    pub(crate) route_path: String,
    pub(crate) guest_path: String,
    pub(crate) host_target: PathBuf,
    pub(crate) mode: StorageWriteMode,
    pub(crate) body: Vec<u8>,
    pub(crate) sync_to_cloud: bool,
}

#[derive(Debug)]
pub(crate) struct StorageBrokerSnapshotRequest {
    pub(crate) volume_id: String,
    pub(crate) source_path: PathBuf,
    pub(crate) snapshot_path: PathBuf,
    pub(crate) completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Debug)]
pub(crate) struct StorageBrokerRestoreRequest {
    pub(crate) volume_id: String,
    pub(crate) snapshot_path: PathBuf,
    pub(crate) destination_path: PathBuf,
    pub(crate) completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StorageWriteMode {
    Overwrite,
    Append,
}

pub(crate) struct ResolvedStorageWriteTarget {
    pub(crate) volume_root: PathBuf,
    pub(crate) guest_path: String,
    pub(crate) host_target: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TtlManagedPath {
    pub(crate) host_path: PathBuf,
    pub(crate) ttl: Duration,
}

pub(crate) static LORA_TRAINING_QUEUE: OnceLock<Arc<LoraTrainingQueue>> = OnceLock::new();
pub(crate) static AI_INFERENCE_JOBS: OnceLock<Arc<Mutex<HashMap<String, AiInferenceJobStatus>>>> =
    OnceLock::new();

// ── Canary deployment types ───────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub(crate) enum DeploymentStrategy {
    #[default]
    Rolling,
    Canary,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CanaryConfig {
    /// Module name of the next version to roll traffic to.
    pub(crate) next_version: String,
    /// Percentage of traffic to shift per evaluation step (1–100).
    #[serde(default = "default_canary_step_weight")]
    pub(crate) step_weight: u32,
    /// Seconds between evaluation steps.
    #[serde(default = "default_canary_interval_secs")]
    pub(crate) interval_secs: u64,
    /// Error rate above which an automatic rollback is triggered (0.0–1.0).
    pub(crate) max_error_rate: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) metrics: Vec<CanaryMetricThreshold>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct CanaryMetricThreshold {
    pub(crate) name: String,
    pub(crate) threshold: String,
}

pub(crate) fn default_canary_step_weight() -> u32 {
    10
}
pub(crate) fn default_canary_interval_secs() -> u64 {
    60
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CanaryPhase {
    Stepping,
    Promoted,
    RolledBack { reason: String },
}

pub(crate) struct CanaryRolloutState {
    pub(crate) route_path: String,
    pub(crate) current_version: String,
    pub(crate) next_version: String,
    /// Current percentage of traffic directed to `next_version` (0–100).
    pub(crate) weight_pct: AtomicU32,
    pub(crate) step_weight: u32,
    pub(crate) interval_secs: u64,
    pub(crate) max_error_rate: f32,
    pub(crate) metric_thresholds: Vec<CanaryMetricThreshold>,
    pub(crate) phase: Mutex<CanaryPhase>,
    /// Cumulative requests routed to `next_version` on this node.
    pub(crate) next_req_count: AtomicU64,
    /// Cumulative 5xx / wasm-trap responses from `next_version` on this node.
    pub(crate) next_err_count: AtomicU64,
    /// Send `true` to stop the evaluator task for this rollout.
    pub(crate) stop_tx: tokio::sync::watch::Sender<bool>,
}

#[allow(clippy::type_complexity)]
pub(crate) static CANARY_ROLLOUTS: OnceLock<Arc<Mutex<HashMap<String, Arc<CanaryRolloutState>>>>> =
    OnceLock::new();

pub(crate) fn canary_rollouts() -> &'static Arc<Mutex<HashMap<String, Arc<CanaryRolloutState>>>> {
    CANARY_ROLLOUTS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub(crate) struct LoraTrainingQueue {
    pub(crate) sender: std::sync::mpsc::Sender<LoraTrainingJob>,
    pub(crate) statuses: Arc<Mutex<HashMap<String, LoraTrainingJobStatus>>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LoraTrainingJob {
    pub(crate) id: String,
    pub(crate) tenant_id: String,
    pub(crate) base_model: String,
    pub(crate) dataset_volume: String,
    pub(crate) dataset_path: String,
    pub(crate) dataset_split: Option<String>,
    pub(crate) rank: u32,
    pub(crate) max_steps: u32,
    pub(crate) seed: Option<u64>,
}

#[derive(Clone, Debug)]
pub(crate) enum LoraTrainingJobStatus {
    Queued,
    Running { step: u32, total: u32 },
    Completed { adapter_path: String },
    Failed { message: String },
}

#[derive(Clone, Debug)]
pub(crate) enum AiInferenceJobStatus {
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
pub(crate) struct VolumeManager {
    pub(crate) volumes: Arc<Mutex<HashMap<String, Arc<ManagedVolume>>>>,
}

pub(crate) struct ManagedVolume {
    pub(crate) id: String,
    pub(crate) route_path: String,
    pub(crate) guest_path: String,
    pub(crate) active_path: PathBuf,
    pub(crate) snapshot_path: PathBuf,
    pub(crate) idle_timeout: Duration,
    pub(crate) storage_broker: Arc<StorageBrokerManager>,
    pub(crate) state: Mutex<ManagedVolumeState>,
    pub(crate) notify: Notify,
}

pub(crate) struct ManagedVolumeState {
    pub(crate) lifecycle: ManagedVolumeLifecycle,
    pub(crate) active_leases: usize,
    pub(crate) generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManagedVolumeLifecycle {
    Active,
    Hibernating,
    OnDisk,
}

pub(crate) struct ManagedVolumeLease {
    pub(crate) volume: Arc<ManagedVolume>,
}

pub(crate) struct RouteVolumeLeaseGuard {
    pub(crate) leases: Vec<ManagedVolumeLease>,
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct IntegrityManifest {
    pub(crate) config_payload: String,
    pub(crate) public_key: String,
    pub(crate) signature: String,
}

#[derive(Default)]
pub(crate) struct BackgroundWorkerManager {
    pub(crate) workers: Mutex<Vec<BackgroundWorkerHandle>>,
}

pub(crate) struct BackgroundWorkerHandle {
    pub(crate) route_path: String,
    pub(crate) stop_requested: Arc<AtomicBool>,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GuestLogRecord {
    pub(crate) level: String,
    pub(crate) target: Option<String>,
    pub(crate) fields: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GuestLogStreamType {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct AsyncLogEntry {
    pub(crate) target_name: String,
    pub(crate) timestamp_unix_ms: u64,
    pub(crate) stream_type: GuestLogStreamType,
    pub(crate) level: String,
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) guest_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) structured_fields: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VolumeType {
    Host,
    Ram,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum VolumeEvictionPolicy {
    Hibernate,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) enum RouteQos {
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
    pub(crate) fn score(self) -> u16 {
        match self {
            Self::RealTime => 100,
            Self::Standard => 50,
            Self::Batch => 10,
        }
    }
}

pub(crate) fn is_default_route_qos(qos: &RouteQos) -> bool {
    *qos == RouteQos::Standard
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ModelDevice {
    #[default]
    Cpu,
    Cuda,
    Metal,
    Npu,
    Tpu,
}

impl ModelDevice {
    #[cfg_attr(not(feature = "ai-inference"), allow(dead_code))]
    pub(crate) fn as_str(&self) -> &'static str {
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
pub(crate) struct IntegrityLayer4Config {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tcp: Vec<IntegrityTcpBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) udp: Vec<IntegrityUdpBinding>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct IntegrityTcpBinding {
    pub(crate) port: u16,
    pub(crate) target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct IntegrityUdpBinding {
    pub(crate) port: u16,
    pub(crate) target: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct IntegrityRoute {
    pub(crate) path: String,
    pub(crate) role: RouteRole,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) dependencies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) requires_credentials: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) middleware: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) allowed_secrets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) targets: Vec<RouteTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resiliency: Option<ResiliencyConfig>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) models: Vec<IntegrityModelBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) domains: Vec<String>,
    #[serde(default)]
    pub(crate) min_instances: u32,
    #[serde(default = "default_max_concurrency")]
    pub(crate) max_concurrency: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) volumes: Vec<IntegrityVolume>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) resource_policy: Option<ResourcePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) runtime: Option<FaaSRuntime>,
    /// Routes flagged here mirror data writes to a cloud endpoint via the existing
    /// `system-faas-cdc` outbox path. Off by default; opting in adds an asynchronous
    /// post-write event emit but no synchronous latency.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) sync_to_cloud: bool,
    /// Route runs inside a hardware Trusted Execution Environment when true. The host
    /// dispatches via `IntegrityConfig::tee_backend` instead of the pooled engine.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) requires_tee: bool,
    /// Route may overflow to peer nodes via `system-faas-mesh-overlay` when the local
    /// accelerator or worker pool is saturated.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) allow_overflow: bool,
    /// Opt-in distributed rate-limiting policy enforced via `system-faas-dist-limiter`.
    /// When `None`, only the local LRU rate limiter applies (the host fails open on a
    /// distributed limiter outage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) distributed_rate_limit: Option<DistributedRateLimitConfig>,
    /// Optional target module or internal URL that receives a fire-and-forget copy
    /// of primary traffic through `system-faas-shadow-proxy`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) shadow_target: Option<String>,
    /// Tenant-specific LoRA adapter to apply on top of the route's foundation model
    /// at inference time. Per-call overrides may be passed via the inference WIT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) adapter_id: Option<String>,
    /// When set, enables an automated canary rollout for this route. The host
    /// gradually shifts traffic from the current module (`name`) to
    /// `canary.next_version` according to the configured step schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) canary: Option<CanaryConfig>,
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
            canary: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AdmissionStrategy {
    #[default]
    FailFast,
    MeshRetry,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct ResourcePolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) min_ram_gb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) min_ram_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) min_vram_mb: Option<u64>,
    /// GPU VRAM reservation for the workload in MiB (scheduler-enforced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) vram_mb: Option<u64>,
    /// Optional GPU device affinity selector (e.g. "cuda:0", "hip:1", or a model substring).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gpu_affinity: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_admission_strategy")]
    pub(crate) admission_strategy: AdmissionStrategy,
}

pub(crate) fn is_default_admission_strategy(strategy: &AdmissionStrategy) -> bool {
    *strategy == AdmissionStrategy::FailFast
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct DistributedRateLimitConfig {
    /// Request count permitted across the entire mesh within `window_seconds`.
    pub(crate) threshold: u32,
    #[serde(default = "default_dist_rate_limit_window")]
    pub(crate) window_seconds: u32,
    #[serde(default, skip_serializing_if = "is_default_dist_rate_limit_scope")]
    pub(crate) scope: DistributedRateLimitScope,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DistributedRateLimitScope {
    #[default]
    Ip,
    Tenant,
    Token,
}

pub(crate) fn is_default_dist_rate_limit_scope(scope: &DistributedRateLimitScope) -> bool {
    *scope == DistributedRateLimitScope::Ip
}

pub(crate) fn default_dist_rate_limit_window() -> u32 {
    60
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct IntegrityBatchTarget {
    pub(crate) name: String,
    pub(crate) module: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) volumes: Vec<IntegrityVolume>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct IntegrityModelBinding {
    pub(crate) alias: String,
    pub(crate) path: String,
    #[serde(default, skip_serializing_if = "is_default_model_device")]
    pub(crate) device: ModelDevice,
    #[serde(default, skip_serializing_if = "is_default_route_qos")]
    pub(crate) qos: RouteQos,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct IntegrityVolume {
    #[serde(
        rename = "type",
        default = "default_volume_type",
        skip_serializing_if = "is_default_volume_type"
    )]
    pub(crate) volume_type: VolumeType,
    pub(crate) host_path: String,
    pub(crate) guest_path: String,
    #[serde(default)]
    pub(crate) readonly: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ttl_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) idle_timeout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) eviction_policy: Option<VolumeEvictionPolicy>,
    /// Route writes to this volume are paged through `system-faas-tde` for AES-256-GCM
    /// encryption at rest. Off by default to preserve native disk speed for routes
    /// that don't need TDE.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) encrypted: bool,
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
pub(crate) enum IntegrityResource {
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

// ── KV-cache configuration ────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum KvCacheEvictionPolicy {
    #[default]
    Lru,
    Lfu,
    Fifo,
}

/// Declares a token KV-cache that is bound to a specific LLM deployment.
/// Writes are only accepted on nodes where `model_ref` is currently hot;
/// the `model_ref` is used as the first segment of every storage key so
/// entries from different models are physically isolated in the ReDB table.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct IntegrityKvCacheConfig {
    /// Logical name for this cache (used in admin APIs and metrics).
    pub(crate) name: String,
    /// Alias of the LLM model this cache is bound to (must match a route's
    /// model alias). Writes are refused with 503 if this model is not
    /// currently loaded on the receiving node.
    pub(crate) model_ref: String,
    /// Maximum TTL in seconds for individual cache entries (`None` = no expiry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) max_ttl_seconds: Option<u64>,
    /// Eviction strategy applied when `max_ttl_seconds` expires on reads.
    #[serde(default)]
    pub(crate) eviction_policy: KvCacheEvictionPolicy,
    /// Isolate entries per tenant (uses the `x-tachyon-tenant` request header
    /// as the tenant segment in storage keys). Defaults to `true`.
    #[serde(default = "default_true")]
    pub(crate) tenant_isolation: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct IntegrityConfig {
    pub(crate) host_address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) advertise_ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tls_address: Option<String>,
    pub(crate) max_stdout_bytes: usize,
    pub(crate) guest_fuel_budget: u64,
    pub(crate) guest_memory_limit_bytes: usize,
    pub(crate) resource_limit_response: String,
    #[serde(default, skip_serializing_if = "IntegrityLayer4Config::is_empty")]
    pub(crate) layer4: IntegrityLayer4Config,
    #[serde(
        default = "default_telemetry_sample_rate",
        skip_serializing_if = "is_default_telemetry_sample_rate"
    )]
    pub(crate) telemetry_sample_rate: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) batch_targets: Vec<IntegrityBatchTarget>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) resources: BTreeMap<String, IntegrityResource>,
    pub(crate) routes: Vec<IntegrityRoute>,
    /// Monotonically increasing version stamp used by the multi-master config sync
    /// path: a node receives a `ConfigUpdateEvent` and pulls the manifest from the
    /// origin only when the advertised version is higher than the local one.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub(crate) config_version: u64,
    /// Outbound endpoint a freshly booted, unenrolled node uses to wait for a PIN
    /// approval from an active mesh node. Optional — clusters that pre-seed
    /// credentials don't need it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) enrollment_endpoint: Option<String>,
    /// Cloud endpoint that `system-faas-cdc` POSTs to when draining the
    /// data-mutation outbox. Optional — air-gapped deployments leave it unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cloud_sync_endpoint: Option<String>,
    /// TEE delegation backend used by routes flagged `requires_tee`. Optional —
    /// without it, a manifest with TEE-flagged routes is rejected by validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tee_backend: Option<TeeBackendConfig>,
    /// Hard cap on memory used by the Wasmtime instance pool. Optional — when unset,
    /// the existing `PoolingAllocationConfig` defaults apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) instance_pool_max_memory_bytes: Option<usize>,
    /// Hex-encoded Ed25519 public keys of peer nodes whose `integrity.lock`
    /// manifests this node should accept in addition to the embedded
    /// boot-time key.  Populated via `PUT /admin/identity/trusted-signers` and
    /// propagated by the config-update gossip path after each seal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) trusted_signers: Vec<String>,
    /// Deployed asset versions, keyed by asset name, values are SemVer strings
    /// (e.g. `"2.4.1"`).  Written by `admin_manifest_bundle_handler` after each
    /// successful bundle apply so subsequent applies can detect whether a higher
    /// compatible version is already present in the cluster.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) asset_versions: BTreeMap<String, String>,
    /// LLM inference KV-caches declared for this node. Each entry binds a
    /// cache to a specific model via `model_ref`; writes are only accepted
    /// when that model is hot on the receiving node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) kv_caches: Vec<IntegrityKvCacheConfig>,
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
            trusted_signers: Vec::new(),
            asset_versions: BTreeMap::new(),
            kv_caches: Vec::new(),
        }
    }
}

pub(crate) fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum TeeBackendConfig {
    /// In-process hardened wasmtime backend with mlocked memory and a self-attested
    /// JWT carrying the host identity. Available on every host; security guarantees
    /// match the host kernel.
    LocalEnclave,
    /// Real Enarx backend. Requires the `enarx` Cargo feature and SGX/SEV-SNP HW.
    Enarx { keep_endpoint: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OutboundTargetKind {
    Internal,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedOutboundTarget {
    pub(crate) url: String,
    pub(crate) kind: OutboundTargetKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedRoute {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) version: Version,
    pub(crate) dependencies: HashMap<String, VersionReq>,
    pub(crate) requires_credentials: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct RouteRegistry {
    pub(crate) by_name: HashMap<String, Vec<ResolvedRoute>>,
    pub(crate) by_path: HashMap<String, ResolvedRoute>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BatchTargetRegistry {
    pub(crate) by_name: HashMap<String, IntegrityBatchTarget>,
}

impl IntegrityLayer4Config {
    pub(crate) fn is_empty(&self) -> bool {
        self.tcp.is_empty() && self.udp.is_empty()
    }
}

#[derive(Debug)]
pub(crate) enum ExecutionError {
    GuestModuleNotFound(GuestModuleNotFound),
    ResourceLimitExceeded {
        kind: ResourceLimitKind,
        detail: String,
    },
    Internal(String),
}

#[derive(Debug, Parser)]
#[command(name = "core-host")]
pub(crate) struct HostCli {
    #[arg(long, value_enum, default_value_t = AccelerationMode::Userspace)]
    pub(crate) accel: AccelerationMode,
    #[command(subcommand)]
    pub(crate) command: Option<HostCommand>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub(crate) enum AccelerationMode {
    #[default]
    Userspace,
    Ebpf,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HostCommand {
    Serve,
    Run(RunCommand),
}

#[derive(Debug, ClapArgs)]
pub(crate) struct RunCommand {
    #[arg(long)]
    pub(crate) manifest: Option<PathBuf>,
    #[arg(long)]
    pub(crate) target: String,
}
