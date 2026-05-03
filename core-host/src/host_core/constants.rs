use super::*;

#[cfg(test)]
pub(crate) const DEFAULT_HOST_ADDRESS: &str = "0.0.0.0:8080";
#[cfg(test)]
pub(crate) const DEFAULT_MAX_STDOUT_BYTES: usize = 64 * 1024;
#[cfg(test)]
pub(crate) const DEFAULT_GUEST_FUEL_BUDGET: u64 = 500_000_000;
#[cfg(test)]
pub(crate) const DEFAULT_GUEST_MEMORY_LIMIT_BYTES: usize = 50 * 1024 * 1024;
#[cfg(test)]
pub(crate) const DEFAULT_RESOURCE_LIMIT_RESPONSE: &str =
    "Execution trapped: Resource limit exceeded";
#[cfg(test)]
pub(crate) const DEFAULT_ROUTE: &str = "/api/guest-example";
#[cfg(test)]
pub(crate) const DEFAULT_SYSTEM_ROUTE: &str = "/metrics";
#[cfg(test)]
pub(crate) const DEFAULT_TLS_ADDRESS: &str = "127.0.0.1:3443";
pub(crate) const ACME_STAGING_MOCK_MODE: &str = "ACME_STAGING_MOCK";
pub(crate) const CERT_MANAGER_GUEST_CERT_DIR: &str = "/app/certs";
pub(crate) const SYSTEM_METERING_ROUTE: &str = "/system/metering";
pub(crate) const SYSTEM_BRIDGE_ROUTE: &str = "/system/bridge";
pub(crate) const SYSTEM_CERT_MANAGER_ROUTE: &str = "/system/cert-manager";
pub(crate) const SYSTEM_GATEWAY_ROUTE: &str = "/system/gateway";
pub(crate) const SYSTEM_GITOPS_BROKER_ROUTE: &str = "/system/gitops-broker";
pub(crate) const SYSTEM_GITOPS_BROKER_MODULE: &str = "system-faas-gitops-broker";
pub(crate) const SYSTEM_LOGGER_ROUTE: &str = "/system/logger";
pub(crate) const SYSTEM_DIST_LIMITER_ROUTE: &str = "/system/dist-limiter";
pub(crate) const SYSTEM_SHADOW_PROXY_ROUTE: &str = "/system/shadow-proxy";
pub(crate) const EMBEDDED_CONFIG_PAYLOAD: &str = env!("FAAS_CONFIG");
pub(crate) const EMBEDDED_PUBLIC_KEY: &str = env!("FAAS_PUBKEY");
pub(crate) const EMBEDDED_SIGNATURE: &str = env!("FAAS_SIGNATURE");
pub(crate) const INTEGRITY_MANIFEST_PATH_ENV: &str = "TACHYON_INTEGRITY_MANIFEST";
pub(crate) const BOOTSTRAP_IF_UNENROLLED_ENV: &str = "TACHYON_BOOTSTRAP_IF_UNENROLLED";
pub(crate) const ENROLLMENT_CERT_PATH_ENV: &str = "TACHYON_ENROLLMENT_CERT_PATH";
pub(crate) const NODE_CERT_PEM_ENV: &str = "TACHYON_NODE_CERT_PEM";
pub(crate) const NODE_KEY_PEM_ENV: &str = "TACHYON_NODE_KEY_PEM";
pub(crate) const DEFAULT_HOP_LIMIT: u32 = 10;
pub(crate) const HOP_LIMIT_HEADER: &str = "x-tachyon-hop-limit";
pub(crate) const COHORT_HEADER: &str = "x-cohort";
pub(crate) const TACHYON_COHORT_HEADER: &str = "x-tachyon-cohort";
pub(crate) const TACHYON_IDENTITY_HEADER: &str = "x-tachyon-identity";
pub(crate) const TACHYON_ORIGINAL_ROUTE_HEADER: &str = "x-tachyon-original-route";
pub(crate) const TACHYON_BUFFER_REPLAY_HEADER: &str = "x-tachyon-buffer-replay";
pub(crate) const MESH_QOS_OVERRIDE_PREFIX: &str = "mesh-qos:";
pub(crate) const TACHYON_SYSTEM_PUBLIC_KEY_ENV: &str = "TACHYON_SYSTEM_PUBLIC_KEY";
pub(crate) const TACHYON_MTLS_ADDRESS_ENV: &str = "TACHYON_MTLS_ADDRESS";
pub(crate) const TACHYON_CONFIG_STORE_DIR_ENV: &str = "TACHYON_CONFIG_STORE_DIR";
pub(crate) const DEFAULT_GITOPS_CONFIG_STORE_PATH: &str = "/var/lib/tachyon/config-store";
pub(crate) const GITOPS_CONFIG_STORE_GUEST_PATH: &str = "/var/lib/tachyon/config-store";
#[cfg(unix)]
pub(crate) const TACHYON_DISCOVERY_DIR_ENV: &str = "TACHYON_DISCOVERY_DIR";
pub(crate) const LOG_QUEUE_CAPACITY: usize = 64_000;
pub(crate) const CONFIG_UPDATE_CHANNEL_CAPACITY: usize = 1_024;
pub(crate) const LOG_BATCH_SIZE: usize = 1_000;
pub(crate) const LOG_BATCH_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
pub(crate) const SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD: usize = 32;
pub(crate) const DEFAULT_ROUTE_MAX_CONCURRENCY: u32 = 100;
#[cfg(test)]
pub(crate) const DEFAULT_ROUTE_VERSION: &str = "0.0.0";
pub(crate) const DEFAULT_TELEMETRY_SAMPLE_RATE: f64 = 0.0;
pub(crate) const TDE_FILE_MAGIC: &[u8] = b"TACHYON-TDE-v1\0";
pub(crate) const TDE_KEY_HEX_ENV: &str = "TDE_KEY_HEX";
pub(crate) const MODEL_BROKER_DIR_ENV: &str = "MODEL_BROKER_DIR";
pub(crate) const AUTOSCALING_TICK_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const VOLUME_GC_TICK_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const DRAINING_REAPER_TICK_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const DRAINING_ROUTE_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const TELEMETRY_EXPORT_QUEUE_CAPACITY: usize = 1024;
pub(crate) const TELEMETRY_EXPORT_BATCH_SIZE: usize = 32;
pub(crate) const UDP_LAYER4_QUEUE_CAPACITY: usize = 256;
pub(crate) const UDP_LAYER4_MAX_WORKERS_PER_LISTENER: usize = 8;
pub(crate) const UDP_LAYER4_MAX_DATAGRAM_SIZE: usize = 65_507;
pub(crate) const BUFFER_RAM_REQUEST_CAPACITY: usize = 32;
pub(crate) const BUFFER_TOTAL_REQUEST_CAPACITY: usize = 256;
pub(crate) const BUFFER_REPLAY_RETRY_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(not(test))]
pub(crate) const BUFFER_RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(test)]
pub(crate) const BUFFER_RESPONSE_WAIT_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const PRESSURE_MONITOR_IDLE_SLEEP_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const PRESSURE_MONITOR_POLL_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(unix)]
pub(crate) const PEER_PRESSURE_STALE_AFTER: Duration = Duration::from_secs(10);
pub(crate) const PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD: usize = 8;
pub(crate) const PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD: usize = 32;
pub(crate) const IDENTITY_TOKEN_TTL: Duration = Duration::from_secs(30);
pub(crate) const IDENTITY_TOKEN_PREFIX: &str = "tachyon.v1";
pub(crate) const KUBERNETES_SERVICE_BASE_URL: &str = "https://kubernetes.default.svc";
pub(crate) const MOCK_K8S_URL_ENV: &str = "TACHYON_MOCK_K8S_URL";
#[cfg(unix)]
pub(crate) const DEFAULT_DISCOVERY_DIR: &str = "/tmp/tachyon/peers";
#[cfg(not(test))]
pub(crate) const ROUTE_CONCURRENCY_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
pub(crate) const ROUTE_CONCURRENCY_WAIT_TIMEOUT: Duration = Duration::from_millis(50);
pub(crate) const DISTRIBUTED_RATE_LIMIT_TIMEOUT: Duration = Duration::from_millis(5);
pub(crate) static DISTRIBUTED_RATE_LIMIT_BYPASS_TOTAL: AtomicU64 = AtomicU64::new(0);
pub(crate) const POOLING_CORE_INSTANCES_MULTIPLIER: u32 = 8;
pub(crate) const POOLING_MEMORIES_MULTIPLIER: u32 = 2;
pub(crate) const POOLING_TABLES_MULTIPLIER: u32 = 2;
pub(crate) const POOLING_INSTANCE_METADATA_BYTES: usize = 1 << 20;
pub(crate) const POOLING_MAX_CORE_INSTANCES_PER_COMPONENT: u32 = 50;
pub(crate) const POOLING_MAX_MEMORIES_PER_COMPONENT: u32 = 8;
pub(crate) const POOLING_MAX_TABLES_PER_COMPONENT: u32 = 8;
pub(crate) const ERR_INTEGRITY_SCHEMA_VIOLATION: &str = "ERR_INTEGRITY_SCHEMA_VIOLATION";

pub(crate) fn default_max_concurrency() -> u32 {
    DEFAULT_ROUTE_MAX_CONCURRENCY
}

#[cfg(test)]
pub(crate) fn default_route_version() -> String {
    DEFAULT_ROUTE_VERSION.to_owned()
}

pub(crate) fn default_telemetry_sample_rate() -> f64 {
    DEFAULT_TELEMETRY_SAMPLE_RATE
}

pub(crate) fn is_default_telemetry_sample_rate(sample_rate: &f64) -> bool {
    (*sample_rate - DEFAULT_TELEMETRY_SAMPLE_RATE).abs() < f64::EPSILON
}

pub(crate) fn unix_timestamp_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system clock is set before the Unix epoch")?
        .as_secs())
}

pub(crate) fn core_store_path(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("tachyon.db")
}

pub(crate) fn buffered_request_spool_dir(manifest_path: &Path) -> PathBuf {
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("buffered-requests")
}

pub(crate) async fn open_core_store_for_manifest(
    manifest_path: &Path,
) -> Result<Arc<store::CoreStore>> {
    let manifest_path = manifest_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        store::CoreStore::open(&core_store_path(&manifest_path)).map(Arc::new)
    })
    .await
    .context("core store initialization task failed")?
}

pub(crate) fn forbidden_error(message: &str) -> String {
    format!("forbidden:{message}")
}

pub(crate) fn default_volume_type() -> VolumeType {
    VolumeType::Host
}

pub(crate) fn is_default_volume_type(volume_type: &VolumeType) -> bool {
    *volume_type == VolumeType::Host
}

pub(crate) fn is_default_model_device(device: &ModelDevice) -> bool {
    *device == ModelDevice::Cpu
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

impl Capabilities {
    pub(crate) const CORE_WASI: u64 = 1 << 0;
    pub(crate) const LEGACY_OCI: u64 = 1 << 1;
    pub(crate) const ACCEL_CUDA: u64 = 1 << 2;
    pub(crate) const ACCEL_OPENVINO: u64 = 1 << 3;
    pub(crate) const ACCEL_TPU: u64 = 1 << 4;
    pub(crate) const NET_LAYER4: u64 = 1 << 5;
    pub(crate) const FEATURE_WEBSOCKETS: u64 = 1 << 6;
    pub(crate) const FEATURE_HTTP3: u64 = 1 << 7;
    pub(crate) const FEATURE_AI_INFERENCE: u64 = 1 << 8;
    pub(crate) const OS_LINUX: u64 = 1 << 9;
    pub(crate) const OS_WINDOWS: u64 = 1 << 10;

    pub(crate) fn detect() -> Self {
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

    pub(crate) fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    pub(crate) fn from_requirement_list(requirements: &[String]) -> Result<Self> {
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

    pub(crate) fn supports(self, required: Self) -> bool {
        (self.mask & required.mask) == required.mask
    }

    pub(crate) fn names(self) -> Vec<String> {
        capability_names_from_mask(self.mask)
    }

    pub(crate) fn missing_names(self, required: Self) -> Vec<String> {
        capability_names_from_mask(required.mask & !self.mask)
    }
}

pub(crate) fn default_route_capabilities() -> Vec<String> {
    vec!["core:wasi".to_owned()]
}

pub(crate) fn capability_flag(value: &str) -> Result<u64> {
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

pub(crate) fn capability_names_from_mask(mask: u64) -> Vec<String> {
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

pub(crate) fn normalize_capabilities(
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

pub(crate) fn is_v1_container_runtime() -> bool {
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
