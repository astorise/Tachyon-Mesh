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
