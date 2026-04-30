use super::*;

pub(crate) struct DrainingRuntime {
    pub(crate) runtime: Arc<RuntimeState>,
    pub(crate) draining_since: Instant,
}

pub(crate) struct TcpLayer4ListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_addr: SocketAddr,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
}

pub(crate) struct UdpLayer4ListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_addr: SocketAddr,
    pub(crate) join_handles: Vec<tokio::task::JoinHandle<()>>,
}

pub(crate) struct HttpsListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_addr: SocketAddr,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
}

pub(crate) struct MtlsGatewayListenerHandle {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) local_addr: SocketAddr,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
}

pub(crate) struct Http3ListenerHandle {
    #[allow(dead_code)]
    pub(crate) local_addr: SocketAddr,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HopLimit(pub(crate) u32);

#[cfg(unix)]
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct UdsPeerMetadata {
    pub(crate) host_id: String,
    pub(crate) ip: String,
    pub(crate) socket_path: String,
    pub(crate) protocols: Vec<String>,
    #[serde(default)]
    pub(crate) pressure_state: PeerPressureState,
    #[serde(default)]
    pub(crate) last_pressure_update_unix_ms: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveredUdsPeer {
    pub(crate) metadata_path: PathBuf,
    pub(crate) socket_path: PathBuf,
    pub(crate) metadata: UdsPeerMetadata,
}

#[cfg(unix)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalUdsEndpoint {
    pub(crate) metadata_path: PathBuf,
    pub(crate) socket_path: PathBuf,
}

#[cfg(unix)]
#[derive(Clone, Default)]
pub(crate) struct UdsFastPathRegistry {
    pub(crate) discovery_dir_override: Arc<Mutex<Option<PathBuf>>>,
    pub(crate) peers: Arc<Mutex<HashMap<String, DiscoveredUdsPeer>>>,
    pub(crate) local_endpoint: Arc<Mutex<Option<LocalUdsEndpoint>>>,
}

#[cfg(not(unix))]
#[derive(Clone, Default)]
pub(crate) struct UdsFastPathRegistry;

#[cfg(unix)]
pub(crate) fn new_uds_fast_path_registry() -> UdsFastPathRegistry {
    UdsFastPathRegistry::default()
}

#[cfg(not(unix))]
pub(crate) fn new_uds_fast_path_registry() -> UdsFastPathRegistry {
    UdsFastPathRegistry
}

pub(crate) struct LegacyHostState {
    pub(crate) wasi: WasiP1Ctx,
    #[cfg(feature = "ai-inference")]
    pub(crate) wasi_nn: WasiNnCtx,
    pub(crate) limits: GuestResourceLimiter,
}

pub(crate) struct ComponentHostState {
    pub(crate) ctx: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: GuestResourceLimiter,
    pub(crate) secrets: SecretAccess,
    pub(crate) runtime_config: IntegrityConfig,
    pub(crate) request_headers: HeaderMap,
    pub(crate) host_identity: Arc<HostIdentity>,
    pub(crate) storage_broker: Arc<StorageBrokerManager>,
    pub(crate) bridge_manager: Arc<BridgeManager>,
    pub(crate) telemetry: TelemetryHandle,
    pub(crate) concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    pub(crate) propagated_headers: Vec<PropagatedHeader>,
    pub(crate) route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    pub(crate) peer_capabilities: PeerCapabilityCache,
    pub(crate) host_capabilities: Capabilities,
    pub(crate) host_load: Arc<HostLoadCounters>,
    pub(crate) outbound_http_client: reqwest::blocking::Client,
    pub(crate) route_path: String,
    pub(crate) route_role: RouteRole,
    #[cfg(feature = "ai-inference")]
    pub(crate) ai_runtime: Option<Arc<ai_inference::AiInferenceRuntime>>,
    #[cfg(feature = "ai-inference")]
    pub(crate) allowed_model_aliases: BTreeSet<String>,
    #[cfg(feature = "ai-inference")]
    pub(crate) adapter_id: Option<String>,
    #[cfg(feature = "ai-inference")]
    pub(crate) accelerator_models: HashMap<u32, LoadedAcceleratorModel>,
    #[cfg(feature = "ai-inference")]
    pub(crate) next_accelerator_model_id: u32,
}

#[cfg(feature = "ai-inference")]
#[derive(Clone, Debug)]
pub(crate) struct LoadedAcceleratorModel {
    pub(crate) alias: String,
    pub(crate) accelerator: ai_inference::AcceleratorKind,
}

pub(crate) struct BatchCommandState {
    pub(crate) ctx: WasiCtx,
    pub(crate) table: ResourceTable,
}

#[derive(Clone)]
pub(crate) struct GuestTelemetryContext {
    pub(crate) handle: TelemetryHandle,
    pub(crate) trace_id: String,
}

pub(crate) struct GuestExecutionContext {
    pub(crate) config: IntegrityConfig,
    pub(crate) sampled_execution: bool,
    pub(crate) runtime_telemetry: TelemetryHandle,
    pub(crate) async_log_sender: mpsc::Sender<AsyncLogEntry>,
    pub(crate) secret_access: SecretAccess,
    pub(crate) request_headers: HeaderMap,
    pub(crate) host_identity: Arc<HostIdentity>,
    pub(crate) storage_broker: Arc<StorageBrokerManager>,
    pub(crate) bridge_manager: Arc<BridgeManager>,
    pub(crate) telemetry: Option<GuestTelemetryContext>,
    pub(crate) concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    pub(crate) propagated_headers: Vec<PropagatedHeader>,
    pub(crate) route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    pub(crate) host_load: Arc<HostLoadCounters>,
    /// In-memory `Arc<Module>` cache shared with the active runtime. The hot
    /// HTTP / L4 paths consult this before the redb-backed `cwasm_cache` to
    /// avoid the `Module::deserialize` cost on every request. Tests fill in
    /// `None`; production code clones it from `RuntimeState::instance_pool`.
    pub(crate) instance_pool: Option<Arc<moka::sync::Cache<PathBuf, Arc<Module>>>>,
    #[cfg(feature = "ai-inference")]
    pub(crate) ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

pub(crate) static BLOCKING_OUTBOUND_HTTP_CLIENT: OnceLock<reqwest::blocking::Client> =
    OnceLock::new();

pub(crate) fn blocking_outbound_http_client() -> reqwest::blocking::Client {
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

pub(crate) struct BackgroundTickRunner {
    pub(crate) function_name: String,
    pub(crate) route_path: String,
    pub(crate) store: Store<ComponentHostState>,
    pub(crate) bindings: BackgroundGuestBindings,
}

pub(crate) enum BackgroundGuestBindings {
    Background(background_component_bindings::BackgroundSystemFaas),
    ControlPlane(control_plane_component_bindings::ControlPlaneFaas),
}

#[derive(Clone, Default)]
pub(crate) struct SecretsVault {
    #[cfg(feature = "secrets-vault")]
    pub(crate) entries: Arc<HashMap<String, String>>,
}

#[cfg_attr(not(feature = "secrets-vault"), allow(dead_code))]
#[derive(Clone, Debug, Default)]
pub(crate) struct SecretAccess {
    pub(crate) allowed_secrets: BTreeSet<String>,
    #[cfg(feature = "secrets-vault")]
    pub(crate) entries: Arc<HashMap<String, String>>,
}

#[derive(Debug)]
pub(crate) struct GuestResourceLimiter {
    pub(crate) max_memory_bytes: usize,
}

pub(crate) type GuestHttpFields = Vec<(String, String)>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GuestRequest {
    pub(crate) method: String,
    pub(crate) uri: String,
    pub(crate) headers: GuestHttpFields,
    pub(crate) body: Bytes,
    pub(crate) trailers: GuestHttpFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GuestHttpResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: GuestHttpFields,
    pub(crate) body: Bytes,
    pub(crate) trailers: GuestHttpFields,
}

pub(crate) struct GuestResponseBody {
    pub(crate) data: Option<Bytes>,
    pub(crate) trailers: Option<HeaderMap>,
    pub(crate) _completion_guard: Option<RouteResponseGuard>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UdpResponseDatagram {
    pub(crate) target: SocketAddr,
    pub(crate) payload: Bytes,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GuestExecutionOutput {
    Http(GuestHttpResponse),
    LegacyStdout(Bytes),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct GuestExecutionOutcome {
    pub(crate) output: GuestExecutionOutput,
    pub(crate) fuel_consumed: Option<u64>,
}

pub(crate) struct RouteExecutionResult {
    pub(crate) response: GuestHttpResponse,
    pub(crate) fuel_consumed: Option<u64>,
    pub(crate) completion_guard: Option<RouteResponseGuard>,
}

pub(crate) type BufferedRouteResult =
    std::result::Result<RouteExecutionResult, (StatusCode, String)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BufferedRequestTier {
    Ram,
    Disk,
}

#[derive(Clone)]
pub(crate) struct BufferedRequestManager {
    pub(crate) disk_dir: PathBuf,
    pub(crate) ram_capacity: usize,
    pub(crate) total_capacity: usize,
    pub(crate) state: Arc<Mutex<BufferedRequestState>>,
    pub(crate) notify: Arc<Notify>,
}

pub(crate) struct BufferedRequestState {
    pub(crate) next_id: u64,
    pub(crate) ram_queue: VecDeque<BufferedMemoryRequest>,
    pub(crate) disk_queue: VecDeque<BufferedDiskRequest>,
}

pub(crate) struct BufferedMemoryRequest {
    pub(crate) id: String,
    pub(crate) request: BufferedRouteRequest,
    pub(crate) completion: oneshot::Sender<BufferedRouteResult>,
}

pub(crate) struct BufferedDiskRequest {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) completion: oneshot::Sender<BufferedRouteResult>,
}

pub(crate) struct BufferedQueueItem {
    pub(crate) id: String,
    pub(crate) request: BufferedRouteRequest,
    pub(crate) completion: oneshot::Sender<BufferedRouteResult>,
    pub(crate) disk_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct BufferedRouteRequest {
    pub(crate) route_path: String,
    pub(crate) selected_module: String,
    pub(crate) method: String,
    pub(crate) uri: String,
    pub(crate) headers: GuestHttpFields,
    pub(crate) body: Vec<u8>,
    pub(crate) trailers: GuestHttpFields,
    pub(crate) hop_limit: u32,
    pub(crate) trace_id: Option<String>,
    pub(crate) sampled_execution: bool,
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
pub(crate) enum RouteLifecycleState {
    Active,
    Draining,
}

pub(crate) struct ActiveRouteRequestGuard {
    pub(crate) control: Arc<RouteExecutionControl>,
}

pub(crate) struct RouteResponseGuard {
    pub(crate) control: Arc<RouteExecutionControl>,
}

pub(crate) struct HostLoadGuard {
    pub(crate) counters: Arc<HostLoadCounters>,
    pub(crate) allocated_pages: usize,
}

#[derive(Clone)]
pub(crate) struct RouteInvocation {
    pub(crate) state: AppState,
    pub(crate) runtime: Arc<RuntimeState>,
    pub(crate) route: IntegrityRoute,
    pub(crate) headers: HeaderMap,
    pub(crate) method: Method,
    pub(crate) uri: Uri,
    pub(crate) body: Bytes,
    pub(crate) trailers: GuestHttpFields,
    pub(crate) hop_limit: HopLimit,
    pub(crate) trace_id: Option<String>,
    pub(crate) sampled_execution: bool,
    pub(crate) selected_module: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteServiceError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
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
    pub(crate) fn new(
        method: impl Into<String>,
        uri: impl Into<String>,
        body: impl Into<Bytes>,
    ) -> Self {
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
    pub(crate) fn new(status: StatusCode, body: impl Into<Bytes>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            trailers: Vec::new(),
        }
    }
}

impl GuestResponseBody {
    pub(crate) fn new(
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
    pub(crate) fn new(control: Arc<RouteExecutionControl>) -> Self {
        control.active_requests.fetch_add(1, Ordering::SeqCst);
        Self { control }
    }

    pub(crate) fn into_response_guard(self) -> RouteResponseGuard {
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
    pub(crate) fn new(counters: Arc<HostLoadCounters>, allocated_pages: usize) -> Self {
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
pub(crate) struct UdpInboundDatagram {
    pub(crate) source: SocketAddr,
    pub(crate) payload: Bytes,
}

#[cfg(feature = "websockets")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HostWebSocketFrame {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    Close,
}

#[cfg(feature = "websockets")]
pub(crate) struct HostWebSocketConnection {
    pub(crate) incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    pub(crate) outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum RouteRole {
    User,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct HeaderMatch {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct RetryPolicy {
    #[serde(default)]
    pub(crate) max_retries: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) retry_on: Vec<u16>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct ResiliencyConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) retry_policy: Option<RetryPolicy>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum FaaSRuntime {
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

pub(crate) fn default_microvm_vcpus() -> u8 {
    1
}

pub(crate) fn default_microvm_memory_mb() -> u32 {
    256
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct RouteTarget {
    pub(crate) module: String,
    #[serde(default)]
    pub(crate) weight: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) websocket: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) match_header: Option<HeaderMatch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) requires: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectedRouteTarget {
    pub(crate) module: String,
    pub(crate) websocket: bool,
    pub(crate) required_capabilities: Vec<String>,
    pub(crate) required_capability_mask: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PropagatedHeader {
    pub(crate) name: String,
    pub(crate) value: String,
}
