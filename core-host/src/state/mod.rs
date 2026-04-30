#![allow(dead_code)]

use crate::core_error::{poisoned_lock, CoreResult};
use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock<'a, T>(
    mutex: &'a Mutex<T>,
    name: &'static str,
) -> CoreResult<MutexGuard<'a, T>> {
    mutex.lock().map_err(|_| poisoned_lock(name))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeGenerationState {
    Active,
    Draining,
}

use super::*;

// Extracted application state ownership.

// Extracted runtime generation state helpers.
impl RuntimeState {
    pub(crate) fn mark_draining(&self, started_at: Instant) {
        for control in self.concurrency_limits.values() {
            control.mark_draining(started_at);
        }
    }

    pub(crate) fn active_request_count(&self) -> usize {
        self.concurrency_limits
            .values()
            .map(|control| control.active_request_count())
            .sum()
    }

    pub(crate) fn draining_route_count(&self) -> usize {
        self.concurrency_limits
            .values()
            .filter(|control| control.lifecycle_state() == RouteLifecycleState::Draining)
            .count()
    }
}

// Extracted application state ownership.
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) runtime: Arc<ArcSwap<RuntimeState>>,
    pub(crate) draining_runtimes: Arc<Mutex<Vec<DrainingRuntime>>>,
    pub(crate) http_client: Client,
    pub(crate) async_log_sender: mpsc::Sender<AsyncLogEntry>,
    pub(crate) secrets_vault: SecretsVault,
    pub(crate) host_identity: Arc<HostIdentity>,
    pub(crate) uds_fast_path: Arc<UdsFastPathRegistry>,
    pub(crate) storage_broker: Arc<StorageBrokerManager>,
    pub(crate) bridge_manager: Arc<BridgeManager>,
    pub(crate) core_store: Arc<store::CoreStore>,
    pub(crate) buffered_requests: Arc<BufferedRequestManager>,
    pub(crate) volume_manager: Arc<VolumeManager>,
    pub(crate) route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    pub(crate) peer_capabilities: PeerCapabilityCache,
    pub(crate) host_capabilities: Capabilities,
    pub(crate) host_load: Arc<HostLoadCounters>,
    pub(crate) memory_governor: Arc<memory_governor::MemoryGovernor>,
    pub(crate) telemetry: TelemetryHandle,
    pub(crate) tls_manager: Arc<tls_runtime::TlsManager>,
    pub(crate) mtls_gateway: Option<Arc<tls_runtime::MtlsGatewayConfig>>,
    pub(crate) auth_manager: Arc<auth::AuthManager>,
    pub(crate) enrollment_manager: Arc<node_enrollment::EnrollmentManager>,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub(crate) manifest_path: PathBuf,
    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    pub(crate) background_workers: Arc<BackgroundWorkerManager>,
}

#[derive(Clone)]
pub(crate) struct RuntimeState {
    pub(crate) engine: Engine,
    pub(crate) metered_engine: Engine,
    pub(crate) config: IntegrityConfig,
    pub(crate) route_registry: Arc<RouteRegistry>,
    #[allow(dead_code)]
    pub(crate) batch_target_registry: Arc<BatchTargetRegistry>,
    pub(crate) concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    /// In-memory cache of `Arc<Module>` keyed by module file path. The
    /// existing redb `cwasm_cache` table eliminates the JIT-compile cost
    /// across host restarts, but every request still pays
    /// `Module::deserialize` (~hundreds of microseconds for typical modules)
    /// when re-reading from redb. This cache amortizes that cost across all
    /// requests within a single runtime generation; on hot reload the cache
    /// is dropped along with the rest of the runtime, so configuration
    /// changes propagate without a stale-module concern.
    pub(crate) instance_pool: Arc<moka::sync::Cache<PathBuf, Arc<Module>>>,
    #[cfg(feature = "ai-inference")]
    pub(crate) ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
}

#[derive(Default)]
pub(crate) struct HostLoadCounters {
    pub(crate) active_instances: AtomicUsize,
    pub(crate) allocated_memory_pages: AtomicUsize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Capabilities {
    pub(crate) mask: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CachedPeerCapabilities {
    pub(crate) capabilities: Vec<String>,
    pub(crate) capability_mask: u64,
    pub(crate) effective_pressure: u8,
}

pub(crate) type PeerCapabilityCache = Arc<Mutex<HashMap<String, CachedPeerCapabilities>>>;
