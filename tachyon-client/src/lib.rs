use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey};
use rand_core::OsRng;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::{ErrorKind, Read, Write},
    net::IpAddr,
    path::PathBuf,
    sync::{OnceLock, RwLock},
};
use sysinfo::System;

const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_RECOVERY_CODES_PATH: &str = "/admin/security/recovery-codes";
const ADMIN_ACCOUNT_SECURITY_PATH: &str = "/admin/security/2fa/regenerate";
const ADMIN_STEP_UP_PATH: &str = "/admin/security/step-up";
const ADMIN_PAT_PATH: &str = "/admin/security/pats";
const ADMIN_VOLUMES_BACKUP_PATH: &str = "/admin/volumes/backup";
const ADMIN_VOLUMES_RESTORE_PATH: &str = "/admin/volumes/restore";
const ADMIN_VOLUMES_BACKUPS_PATH: &str = "/admin/volumes/backups";
const ADMIN_ENROLLMENT_START_PATH: &str = "/admin/enrollment/start";
const ADMIN_IAM_USERS_PATH: &str = "/admin/iam/users";
const ADMIN_IAM_GROUPS_PATH: &str = "/admin/iam/groups";
const ADMIN_MANIFEST_BUNDLE_PATH: &str = "/admin/manifest/bundle";
const ADMIN_MANIFEST_PATH: &str = "/admin/manifest";
const ADMIN_METRICS_PATH: &str = "/admin/metrics";
const ADMIN_CANARY_PATH: &str = "/admin/canary";
const ADMIN_SCHEMA_MANIFEST_PATH: &str = "/admin/schema/manifest";
const ADMIN_KV_PATH: &str = "/admin/kv";
const ADMIN_LOGS_PATH: &str = "/admin/logs";
const ADMIN_SHADOW_DIFFS_PATH: &str = "/admin/shadow/diffs";
const ADMIN_CHAOS_SCENARIOS_PATH: &str = "/admin/chaos/scenarios";
const ADMIN_ASSET_UPLOAD_PATH: &str = "/admin/assets";
const AUTH_SIGNUP_VALIDATE_PATH: &str = "/auth/signup/validate-token";
const AUTH_SIGNUP_STAGE_PATH: &str = "/auth/signup/stage";
const AUTH_SIGNUP_FINALIZE_PATH: &str = "/auth/signup/finalize";
const AUTH_LOGIN_STAGE_PATH: &str = "/auth/login/stage";
const AUTH_LOGIN_FINALIZE_PATH: &str = "/auth/login/finalize";
const EXPECTED_HASH_HEADER: &str = "x-tachyon-expected-sha256";
const ADMIN_MODEL_INIT_PATH: &str = "/admin/models/init";
const ADMIN_MODEL_UPLOAD_PATH: &str = "/admin/models/upload";
const ADMIN_MODEL_COMMIT_PATH: &str = "/admin/models/commit";
const ADMIN_NODES_PATH: &str = "/admin/nodes";
const ADMIN_SYSTEMS_REGISTERED_PATH: &str = "/admin/systems/registered";
const ADMIN_SYSTEMS_DEPLOYED_PATH: &str = "/admin/systems/deployed";
const MODEL_CHUNK_BYTES: usize = 16 * 1024 * 1024;
const MODEL_PREP_PROGRESS_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct InstanceConfig {
    pub url: String,
    pub token: String,
    pub mtls_cert: Option<Vec<u8>>,
    pub mtls_key: Option<Vec<u8>>,
}

static CONNECTION_STATE: OnceLock<RwLock<Option<InstanceConfig>>> = OnceLock::new();
static SESSION_OPERATOR: OnceLock<RwLock<Option<String>>> = OnceLock::new();
static HTTP_CLIENT_CACHE: OnceLock<RwLock<BTreeMap<String, reqwest::Client>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthLoginResponse {
    pub username: String,
    pub endpoint: String,
    pub requires_mfa: bool,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationTokenClaims {
    pub subject: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedSignupSession {
    pub session_id: String,
    pub username: String,
    pub provisioning_uri: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamUserSummary {
    pub username: String,
    #[serde(default)]
    pub first_name: String,
    #[serde(default)]
    pub last_name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub disabled_at: Option<u64>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub last_login_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamUserUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_groups: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_roles: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_roles: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamGroupSummary {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamGroupInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IamAuditEntry {
    pub timestamp: u64,
    pub actor: String,
    #[serde(default)]
    pub target_user: Option<String>,
    #[serde(default)]
    pub target_group: Option<String>,
    pub action: String,
    pub outcome: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct IntegrityManifest {
    config_payload: String,
    public_key: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
struct SealedConfig {
    #[serde(default)]
    routes: Vec<serde_json::Value>,
    #[serde(default)]
    batch_targets: Vec<serde_json::Value>,
    #[serde(default)]
    resources: std::collections::BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    kv_caches: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct RecoveryCodeResponse {
    codes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct IssuePatRequest<'a> {
    name: &'a str,
    scopes: &'a [String],
    ttl_days: u32,
}

#[derive(Debug, Deserialize)]
struct IssuePatResponse {
    token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AvailableAiModel {
    pub id: String,
    pub alias: String,
    pub engine: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelListResponse {
    data: Vec<OpenAiModelResponse>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelResponse {
    id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AdminEnrollmentStartRequest<'a> {
    node_public_key: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminEnrollmentStartResponse {
    session_id: String,
    pin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrollmentInvite {
    pub session_id: String,
    pub invite_token: String,
    pub qr_payload: String,
}

#[derive(Debug, Serialize)]
struct RecoveryCodeRequest<'a> {
    username: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ValidateRegistrationTokenRequest<'a> {
    token: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StageLoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizeLoginRequest<'a> {
    session_id: &'a str,
    totp_code: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedLoginResponse {
    session_id: String,
    username: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StageSignupRequest<'a> {
    token: &'a str,
    first_name: &'a str,
    last_name: &'a str,
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizeEnrollmentRequest<'a> {
    session_id: &'a str,
    totp_code: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinalizeEnrollmentResponse {
    token: String,
    username: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepUpSessionRequest<'a> {
    totp_code: &'a str,
}

#[derive(Debug, Deserialize)]
struct AssetUploadResponse {
    asset_uri: String,
}

#[derive(Debug, Serialize)]
struct InitModelUploadRequest<'a> {
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    alias: Option<&'a str>,
    files: Vec<InitModelUploadFile>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InitModelUploadFile {
    path: String,
    size_bytes: u64,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct InitModelUploadResponse {
    upload_id: String,
}

#[derive(Debug, Deserialize)]
struct CommitModelUploadResponse {
    model_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshGraphSnapshot {
    pub source: String,
    pub status: String,
    pub routes: Vec<MeshRouteSummary>,
    pub batch_targets: Vec<String>,
}

// ── Topology graph types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyNodeSpec {
    pub id: String,
    pub node_type: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub data: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyEdgeSpec {
    pub id: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyGraphSpec {
    pub nodes: Vec<TopologyNodeSpec>,
    pub edges: Vec<TopologyEdgeSpec>,
    pub source: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshRouteSummary {
    pub path: String,
    pub name: String,
    pub role: String,
    pub target_count: usize,
    pub requires_tee: bool,
    pub encrypted_volume_count: usize,
    /// True when the route has no explicit scopes block (allow-all sentinel).
    pub allow_all_scopes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshResource {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub pending: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_methods: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_constraint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyConfigurationOutcome {
    pub success: bool,
    pub message: String,
    pub staged: bool,
    pub requires_seal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealApplyOutcome {
    pub success: bool,
    pub message: String,
    pub config_version: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshResourceInput {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub target: String,
    #[serde(default)]
    pub allowed_methods: Option<Vec<String>>,
    #[serde(default)]
    pub version_constraint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuStats {
    pub id: String,
    pub model: String,
    pub vram_total_mb: u64,
    pub vram_used_mb: u64,
    pub compute_utilization: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveSystem {
    pub slug: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeCapabilities {
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub accelerators: Vec<String>,
    pub gpus: Vec<GpuStats>,
    pub active_systems: Vec<ActiveSystem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnrolledNode {
    pub node_id: String,
    pub public_key: String,
    pub status: String,
    pub approved_at: u64,
    pub last_seen: u64,
    pub region: Option<String>,
    pub zone: Option<String>,
    pub capabilities: NodeCapabilities,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredSystem {
    pub slug: String,
    pub crate_name: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedSystemNode {
    pub node_id: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedSystem {
    pub slug: String,
    pub catalog_version: String,
    pub status: String,
    pub host_node_count: usize,
    pub has_drift: bool,
    pub nodes: Vec<DeployedSystemNode>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterHardwareSummary {
    pub source: String,
    pub enrolled_count: usize,
    pub online_count: usize,
    pub stale_count: usize,
    pub total_ram_mb: u64,
    pub gpu_count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterFeatureSet {
    pub has_enrolled_nodes: bool,
    pub has_fleet: bool,
    pub has_ai: bool,
    pub has_routing: bool,
    pub has_resilience: bool,
    pub has_identity: bool,
    pub has_rbac: bool,
    pub has_storage: bool,
    pub has_observability: bool,
    pub has_supply_chain: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareStatus {
    pub total_ram_mb: u64,
    pub available_ram_mb: u64,
    pub accelerators: Vec<String>,
    pub gpus: Vec<GpuStats>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwarePolicy {
    #[serde(default)]
    pub accelerators: Vec<String>,
    #[serde(default)]
    pub min_ram_mb: Option<u64>,
    #[serde(default)]
    pub min_ram_gb: Option<u64>,
    #[serde(default)]
    pub min_vram_mb: Option<u64>,
    #[serde(default)]
    pub qos_class: Option<String>,
    #[serde(default)]
    pub admission_strategy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareValidation {
    pub approved: bool,
    pub reason: String,
    pub available_ram_mb: u64,
    pub required_ram_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DryRunManifestReport {
    pub valid: bool,
    pub config_version: u64,
    pub route_count: usize,
    pub batch_target_count: usize,
    pub resource_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetrics {
    pub source: String,
    pub error_rate: f64,
    pub p50_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub queue_depth: u64,
    #[serde(default)]
    pub vram_utilization_pct: u8,
    #[serde(default)]
    pub ram_offload_active: bool,
    #[serde(default)]
    pub scope_denial_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowDiff {
    pub route: String,
    pub request_id: String,
    pub divergence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_status: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChaosScenarioRequest {
    pub scenario: String,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChaosScenarioOutcome {
    pub accepted: bool,
    pub scenario: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MfaSessionToken {
    pub mfa_session_token: String,
    pub expires_at: u64,
}

fn connection_state() -> &'static RwLock<Option<InstanceConfig>> {
    CONNECTION_STATE.get_or_init(|| RwLock::new(None))
}

fn operator_state() -> &'static RwLock<Option<String>> {
    SESSION_OPERATOR.get_or_init(|| RwLock::new(None))
}

fn is_not_found(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .map(|e| e.kind() == std::io::ErrorKind::NotFound)
            .unwrap_or(false)
    })
}

pub fn workspace_root() -> PathBuf {
    // Candidates tried in priority order; the first directory that actually
    // contains an integrity.lock wins.  This avoids stale env-var overrides
    // (e.g. TACHYON_WORKSPACE_ROOT set to app_local_data_dir when the lockfile
    // hasn't been written there yet) from hiding a perfectly valid lockfile in
    // another location.

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 1. Explicit override (Tauri setup, system admin, CI).
    if let Some(root) = std::env::var_os("TACHYON_WORKSPACE_ROOT") {
        candidates.push(PathBuf::from(root));
    }

    // 2. Directory of the running executable (works for bare binary invocations
    //    and for packaged Tauri apps when core-host writes next to itself).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.to_path_buf());
        }
    }

    // 3. Compile-time workspace root (correct for `cargo run` / dev builds).
    if let Some(dev_root) = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent() {
        candidates.push(dev_root.to_path_buf());
    }

    // Return the first candidate that holds an integrity.lock.
    for dir in &candidates {
        if dir.join("integrity.lock").exists() {
            return dir.clone();
        }
    }

    // No lockfile found anywhere — return the first candidate so callers get a
    // meaningful NotFound error rather than an empty path.
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

pub async fn read_lockfile() -> Result<String> {
    let lockfile_path = workspace_root().join("integrity.lock");
    tokio::fs::read_to_string(&lockfile_path)
        .await
        .with_context(|| format!("failed to read {}", lockfile_path.display()))
}

pub async fn get_engine_status() -> Result<String> {
    if let Some(config) = current_connection() {
        return fetch_remote_status(&config).await;
    }

    match read_lockfile().await {
        Ok(raw) => parse_engine_status(&raw),
        Err(error) if is_not_found(&error) => Ok("offline".to_owned()),
        Err(error) => Err(error),
    }
}

pub fn read_local_hardware_status() -> HardwareStatus {
    let mut system = System::new();
    system.refresh_memory();
    let mut accelerators = vec!["cpu".to_owned()];
    let mut gpus = Vec::new();

    // Detect NVIDIA GPUs from CUDA_VISIBLE_DEVICES; ROCm GPUs from HIP_VISIBLE_DEVICES.
    // Without a GPU management library, VRAM values are reported as zero until the
    // AI inference runtime updates the memory governor and exposes them via /admin/metrics.
    let cuda = std::env::var("CUDA_VISIBLE_DEVICES").ok();
    let hip = std::env::var("HIP_VISIBLE_DEVICES").ok();

    if let Some(ref devices) = cuda {
        accelerators.push("gpu".to_owned());
        for (idx, device_id) in devices.split(',').filter(|s| !s.is_empty()).enumerate() {
            gpus.push(GpuStats {
                id: format!("cuda:{device_id}"),
                model: format!("NVIDIA GPU #{}", idx),
                vram_total_mb: 0,
                vram_used_mb: 0,
                compute_utilization: 0.0,
            });
        }
        if gpus.is_empty() {
            // "all" or unset count — report one entry
            gpus.push(GpuStats {
                id: "cuda:0".to_owned(),
                model: "NVIDIA GPU".to_owned(),
                vram_total_mb: 0,
                vram_used_mb: 0,
                compute_utilization: 0.0,
            });
        }
    } else if let Some(ref devices) = hip {
        accelerators.push("gpu".to_owned());
        for (idx, device_id) in devices.split(',').filter(|s| !s.is_empty()).enumerate() {
            gpus.push(GpuStats {
                id: format!("hip:{device_id}"),
                model: format!("AMD GPU #{}", idx),
                vram_total_mb: 0,
                vram_used_mb: 0,
                compute_utilization: 0.0,
            });
        }
    }

    HardwareStatus {
        total_ram_mb: system.total_memory() / 1024 / 1024,
        available_ram_mb: system.available_memory() / 1024 / 1024,
        accelerators,
        gpus,
    }
}

pub fn validate_hardware_policy(policy: &HardwarePolicy) -> HardwareValidation {
    let status = read_local_hardware_status();
    let required_ram_mb = policy
        .min_ram_mb
        .unwrap_or(0)
        .max(policy.min_ram_gb.unwrap_or(0).saturating_mul(1024));
    let missing_accelerator = policy
        .accelerators
        .iter()
        .find(|accelerator| {
            !status
                .accelerators
                .iter()
                .any(|available| available.eq_ignore_ascii_case(accelerator))
        })
        .cloned();
    let approved = required_ram_mb <= status.available_ram_mb && missing_accelerator.is_none();
    let reason = if let Some(accelerator) = missing_accelerator {
        format!("accelerator `{accelerator}` is not available on this node")
    } else if required_ram_mb > status.available_ram_mb {
        format!(
            "requires {required_ram_mb} MiB RAM but only {} MiB are available",
            status.available_ram_mb
        )
    } else {
        "hardware policy is admissible on this node".to_owned()
    };
    HardwareValidation {
        approved,
        reason,
        available_ram_mb: status.available_ram_mb,
        required_ram_mb,
    }
}

pub async fn get_mesh_graph() -> Result<MeshGraphSnapshot> {
    let current = current_connection();
    let source = current
        .as_ref()
        .map(|config| config.url.clone())
        .unwrap_or_else(|| "workspace://local".to_owned());

    // When there is a remote connection, fetch the live sealed manifest so the
    // Overview panel can count WASM instances from actual deployed routes.
    if let Some(config) = current.as_ref() {
        let status = match fetch_remote_status(config).await {
            Ok(s) => s,
            Err(_) => "offline".to_owned(),
        };
        let sealed = get_admin_json::<SealedConfig>(ADMIN_MANIFEST_PATH)
            .await
            .ok();
        return Ok(MeshGraphSnapshot {
            source,
            status,
            routes: sealed
                .as_ref()
                .map(|s| s.routes.iter().filter_map(parse_route_summary).collect())
                .unwrap_or_default(),
            batch_targets: sealed
                .as_ref()
                .map(|s| {
                    s.batch_targets
                        .iter()
                        .filter_map(batch_target_name)
                        .collect()
                })
                .unwrap_or_default(),
        });
    }

    // No remote connection — fall back to local integrity.lock.
    let raw_lockfile = match read_lockfile().await {
        Ok(raw) => raw,
        Err(error) if is_not_found(&error) => {
            return Ok(MeshGraphSnapshot {
                source,
                status: "offline".to_owned(),
                routes: Vec::new(),
                batch_targets: Vec::new(),
            });
        }
        Err(error) => return Err(error),
    };

    let manifest: IntegrityManifest =
        serde_json::from_str(&raw_lockfile).context("failed to parse integrity.lock manifest")?;
    let sealed_config: SealedConfig = serde_json::from_str(&manifest.config_payload)
        .context("failed to parse sealed config payload")?;
    let status = parse_engine_status(&raw_lockfile).unwrap_or_else(|_| "offline".to_owned());

    Ok(MeshGraphSnapshot {
        source,
        status,
        routes: sealed_config
            .routes
            .iter()
            .filter_map(parse_route_summary)
            .collect(),
        batch_targets: sealed_config
            .batch_targets
            .iter()
            .filter_map(batch_target_name)
            .collect(),
    })
}

pub async fn list_enrolled_nodes() -> Result<Vec<EnrolledNode>> {
    match get_admin_json::<Vec<EnrolledNode>>(ADMIN_NODES_PATH).await {
        Ok(nodes) => Ok(nodes),
        Err(error) if current_connection().is_none() || error.to_string().contains("no active") => {
            Ok(Vec::new())
        }
        Err(error) => Err(error),
    }
}

pub async fn get_cluster_features() -> Result<ClusterFeatureSet> {
    use std::collections::HashSet;
    let nodes = list_enrolled_nodes().await?;
    let slugs: HashSet<&str> = nodes
        .iter()
        .flat_map(|n| {
            n.capabilities
                .active_systems
                .iter()
                .map(|s| s.slug.as_str())
        })
        .collect();

    Ok(ClusterFeatureSet {
        has_enrolled_nodes: !nodes.is_empty(),
        has_fleet: nodes.len() > 1,
        has_ai: slugs.contains("model-broker") || slugs.contains("buffer"),
        has_routing: slugs.contains("gateway") || slugs.contains("mesh-overlay"),
        has_resilience: slugs.contains("shadow-proxy") || slugs.contains("dist-limiter"),
        has_identity: slugs.contains("authn"),
        has_rbac: slugs.contains("authz"),
        has_storage: slugs.contains("s3-proxy") || slugs.contains("storage-broker"),
        has_observability: slugs.contains("otel")
            || slugs.contains("prom")
            || slugs.contains("logger"),
        has_supply_chain: slugs.contains("registry") || slugs.contains("gitops-broker"),
    })
}

pub async fn get_node_capabilities(node_id: &str) -> Result<NodeCapabilities> {
    let nodes = list_enrolled_nodes().await?;
    Ok(nodes
        .into_iter()
        .find(|node| node.node_id == node_id)
        .map(|node| node.capabilities)
        .unwrap_or_default())
}

pub async fn list_registered_systems() -> Result<Vec<RegisteredSystem>> {
    match get_admin_json::<Vec<RegisteredSystem>>(ADMIN_SYSTEMS_REGISTERED_PATH).await {
        Ok(systems) => Ok(systems),
        Err(error) if current_connection().is_none() || error.to_string().contains("no active") => {
            Ok(read_local_system_manifest())
        }
        Err(error) => Err(error),
    }
}

pub async fn list_deployed_systems() -> Result<Vec<DeployedSystem>> {
    match get_admin_json::<Vec<DeployedSystem>>(ADMIN_SYSTEMS_DEPLOYED_PATH).await {
        Ok(systems) => Ok(systems),
        Err(error) if current_connection().is_none() || error.to_string().contains("no active") => {
            let nodes = list_enrolled_nodes().await.unwrap_or_default();
            let registered = read_local_system_manifest();
            Ok(registered
                .into_iter()
                .map(|system| deployed_system_from_nodes(&system, &nodes))
                .collect())
        }
        Err(error) => Err(error),
    }
}

pub async fn get_cluster_hardware_summary() -> Result<ClusterHardwareSummary> {
    let nodes = list_enrolled_nodes().await?;
    Ok(ClusterHardwareSummary {
        source: current_connection()
            .map(|config| config.url)
            .unwrap_or_else(|| "offline".to_owned()),
        enrolled_count: nodes.len(),
        online_count: nodes.iter().filter(|node| node.status == "online").count(),
        stale_count: nodes.iter().filter(|node| node.status == "stale").count(),
        total_ram_mb: nodes
            .iter()
            .map(|node| node.capabilities.total_ram_mb)
            .sum(),
        gpu_count: nodes.iter().map(|node| node.capabilities.gpus.len()).sum(),
    })
}

// ── Topology graph — full component view with real backend data ───────────────

/// Load a SealedConfig from the local integrity.lock, returning (config, status).
/// Returns an offline TopologyGraphSpec error-value when the file is missing.
async fn load_sealed_config_from_file(source: &str) -> Result<(SealedConfig, String)> {
    let raw = match read_lockfile().await {
        Ok(raw) => raw,
        Err(error) if is_not_found(&error) => {
            // Return an offline sentinel via an error that get_topology_graph handles.
            anyhow::bail!("__offline__");
        }
        Err(error) => return Err(error),
    };
    let manifest: IntegrityManifest =
        serde_json::from_str(&raw).context("failed to parse integrity.lock")?;
    let sealed: SealedConfig = serde_json::from_str(&manifest.config_payload)
        .context("failed to parse sealed config payload")?;
    let status = parse_engine_status(&raw).unwrap_or_else(|_| "offline".to_owned());
    let _ = source;
    Ok((sealed, status))
}

pub async fn get_topology_graph() -> Result<TopologyGraphSpec> {
    let current = current_connection();
    let source = current
        .as_ref()
        .map(|c| c.url.clone())
        .unwrap_or_else(|| "workspace://local".to_owned());

    // ── Prefer the live cluster config over a local file ─────────────────────
    // When connected, GET /admin/manifest returns the active IntegrityConfig
    // directly from the running node's memory — no filesystem path resolution
    // required, and it works even when the client runs on a different machine.
    let (sealed, status) = if let Some(config) = current.as_ref() {
        match get_admin_json::<SealedConfig>(ADMIN_MANIFEST_PATH).await {
            Ok(live_config) => {
                let status = fetch_remote_status(config)
                    .await
                    .unwrap_or_else(|_| "degraded".to_owned());
                (live_config, status)
            }
            Err(_) => {
                // Connected but manifest endpoint unreachable — fall back to
                // local file (e.g. older node version without GET /admin/manifest).
                load_sealed_config_from_file(&source).await?
            }
        }
    } else {
        // No remote connection — read local integrity.lock.
        match load_sealed_config_from_file(&source).await {
            Ok(pair) => pair,
            Err(error) if error.to_string().contains("__offline__") => {
                return Ok(TopologyGraphSpec {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    source,
                    status: "offline".to_owned(),
                });
            }
            Err(error) => return Err(error),
        }
    };

    // ── Node construction — two-pass layout ──────────────────────────────────
    // Pass 1: collect (id, node_type, label, data) without positions.
    // Pass 2: compute base rows from actual counts, then assign (x, y).
    let type_order = [
        "endpoint",
        "system-faas",
        "custom-wasm",
        "llm",
        "kv-cache",
        "message-broker",
        "storage",
        "external-resource",
    ];

    struct PendingNode {
        id: String,
        node_type: String,
        label: String,
        data: std::collections::HashMap<String, String>,
    }

    let mut pending: Vec<PendingNode> = Vec::new();
    let mut edges: Vec<TopologyEdgeSpec> = Vec::new();
    let mut edge_counter: usize = 0;
    let mut pending_type_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    // Map route path → node id for edge construction.
    let mut path_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Routes → endpoint / system-faas / llm / custom-wasm nodes
    for route in &sealed.routes {
        let Some(path) = route.get("path").and_then(|v| v.as_str()) else {
            continue;
        };
        let name = route
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| path.trim_start_matches('/'));
        let role = route.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let has_models = route
            .get("models")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        let module_name = route
            .get("module")
            .and_then(|v| v.as_str())
            .or_else(|| {
                route
                    .get("targets")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .and_then(|t| t.get("module"))
                    .and_then(|v| v.as_str())
            })
            .unwrap_or("");
        let is_wasm_module =
            module_name.ends_with(".wasm") || module_name.starts_with("tachyon://");

        // For user routes backed by a WASM module, emit both an endpoint node
        // (the HTTP entry point) and a custom-wasm node (the backing module),
        // connected by an edge.  This gives the canonical two-tier view where
        // the gateway/endpoint is clearly distinct from the WASM implementation.
        let emit_endpoint_pair = role == "user" && is_wasm_module && !has_models;

        let primary_node_type = if has_models {
            "llm"
        } else if role == "system" {
            "system-faas"
        } else if is_wasm_module {
            "custom-wasm"
        } else {
            "endpoint"
        };

        // Endpoint node (always emitted for user routes).
        let endpoint_id = format!("route:{path}");
        path_to_id.insert(path.to_owned(), endpoint_id.clone());

        if emit_endpoint_pair || primary_node_type == "endpoint" {
            let protocol = if path.starts_with("/wss") || path.starts_with("/ws") {
                "WS"
            } else if path.starts_with("/grpc") {
                "gRPC"
            } else {
                "HTTP"
            };
            let mut ep_data: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            ep_data.insert("protocol".to_owned(), protocol.to_owned());
            *pending_type_counts
                .entry("endpoint".to_owned())
                .or_insert(0) += 1;
            pending.push(PendingNode {
                id: endpoint_id.clone(),
                node_type: "endpoint".to_owned(),
                label: path.to_owned(),
                data: ep_data,
            });
        }

        // Backing node (system-faas / custom-wasm / llm).
        if primary_node_type != "endpoint" {
            let wasm_id = if emit_endpoint_pair {
                format!("wasm:{name}")
            } else {
                endpoint_id.clone()
            };

            let mut data: std::collections::HashMap<String, String> =
                std::collections::HashMap::new();
            match primary_node_type {
                "llm" => {
                    if let Some(alias) = route
                        .get("models")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|m| m.get("alias").and_then(|v| v.as_str()))
                    {
                        data.insert("modelName".to_owned(), alias.to_owned());
                    }
                }
                "system-faas" => {
                    data.insert("component".to_owned(), name.to_owned());
                }
                "custom-wasm" => {
                    data.insert("capabilityName".to_owned(), name.to_owned());
                    let version = route
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("^1.0.0");
                    data.insert("semver".to_owned(), version.to_owned());
                    if !module_name.is_empty() {
                        data.insert("assetSource".to_owned(), module_name.to_owned());
                    }
                }
                _ => {}
            }

            *pending_type_counts
                .entry(primary_node_type.to_owned())
                .or_insert(0) += 1;
            pending.push(PendingNode {
                id: wasm_id.clone(),
                node_type: primary_node_type.to_owned(),
                label: name.to_owned(),
                data,
            });

            // Edge: endpoint → backing module.
            if emit_endpoint_pair {
                edge_counter += 1;
                edges.push(TopologyEdgeSpec {
                    id: format!("e{edge_counter}"),
                    from: endpoint_id.clone(),
                    to: wasm_id,
                });
            }
        }

        // Dependency edges (route path → other route path).
        if let Some(deps) = route.get("dependencies").and_then(|v| v.as_object()) {
            for dep_path in deps.keys() {
                edge_counter += 1;
                edges.push(TopologyEdgeSpec {
                    id: format!("e{edge_counter}"),
                    from: endpoint_id.clone(),
                    to: format!("route:{dep_path}"),
                });
            }
        }
    }

    // kv_caches → kv-cache nodes
    for cache in &sealed.kv_caches {
        let Some(name) = cache.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let model_ref = cache
            .get("model_ref")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let capacity = cache
            .get("capacity_mb")
            .and_then(|v| v.as_u64())
            .map(|mb| (mb / 1024).to_string())
            .unwrap_or_default();

        let id = format!("kvcache:{name}");
        let mut data: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        if !capacity.is_empty() {
            data.insert("capacityGb".to_owned(), capacity);
        }
        if !model_ref.is_empty() {
            data.insert("modelRef".to_owned(), model_ref.to_owned());
        }

        *pending_type_counts
            .entry("kv-cache".to_owned())
            .or_insert(0) += 1;
        pending.push(PendingNode {
            id: id.clone(),
            node_type: "kv-cache".to_owned(),
            label: name.to_owned(),
            data,
        });

        // Edge: kv-cache depends on its bound LLM node.
        if !model_ref.is_empty() {
            if let Some(llm_id) = path_to_id.values().find(|v| v.contains(model_ref)) {
                edge_counter += 1;
                edges.push(TopologyEdgeSpec {
                    id: format!("e{edge_counter}"),
                    from: id.clone(),
                    to: llm_id.clone(),
                });
            }
        }
    }

    // External resources → external-resource nodes
    for (name, resource) in &sealed.resources {
        let target = resource
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let id = format!("resource:{name}");
        let mut data: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        data.insert("targetUrl".to_owned(), target.to_owned());

        *pending_type_counts
            .entry("external-resource".to_owned())
            .or_insert(0) += 1;
        pending.push(PendingNode {
            id,
            node_type: "external-resource".to_owned(),
            label: name.clone(),
            data,
        });
    }

    // Pass 2: build layout from actual counts, then assign positions.
    let layout = TopologyLayout::build(&type_order, &pending_type_counts);
    let mut type_counters: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut nodes: Vec<TopologyNodeSpec> = Vec::new();
    for p in pending {
        let (x, y) = layout.position(&p.node_type, &mut type_counters);
        nodes.push(TopologyNodeSpec {
            id: p.id,
            node_type: p.node_type,
            label: p.label,
            x,
            y,
            data: p.data,
        });
    }

    Ok(TopologyGraphSpec {
        nodes,
        edges,
        source,
        status,
    })
}

/// Two-pass topology layout: precompute base rows from actual node counts so
/// that an overflowing type never overlaps the band of the next type.
struct TopologyLayout {
    base_rows: std::collections::HashMap<String, i32>,
    max_per_row: i32,
}

impl TopologyLayout {
    fn build(type_order: &[&str], counts: &std::collections::HashMap<String, usize>) -> Self {
        const MAX_PER_ROW: i32 = 5;
        let mut base_rows = std::collections::HashMap::new();
        let mut current_row: i32 = 0;
        for &t in type_order {
            let count = *counts.get(t).unwrap_or(&0) as i32;
            if count > 0 {
                base_rows.insert(t.to_owned(), current_row);
                current_row += (count + MAX_PER_ROW - 1) / MAX_PER_ROW;
            }
        }
        Self {
            base_rows,
            max_per_row: MAX_PER_ROW,
        }
    }

    fn position(
        &self,
        node_type: &str,
        counters: &mut std::collections::HashMap<String, usize>,
    ) -> (i32, i32) {
        let index = *counters.entry(node_type.to_owned()).or_insert(0);
        *counters.get_mut(node_type).expect("just inserted") += 1;
        let base_row = *self.base_rows.get(node_type).unwrap_or(&0);
        let col = (index as i32) % self.max_per_row;
        let sub_row = (index as i32) / self.max_per_row;
        let x = 40 + col * 300;
        let y = 40 + (base_row + sub_row) * 140;
        (x, y)
    }
}

const OVERLAY_FILE_NAME: &str = "tachyon.resources.json";

fn overlay_path() -> PathBuf {
    workspace_root().join(OVERLAY_FILE_NAME)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ResourceOverlayFile {
    #[serde(default)]
    resources: Vec<MeshResource>,
    #[serde(default)]
    configurations: Vec<StagedConfiguration>,
    #[serde(default)]
    system_routes: Vec<StagedSystemRoute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedConfiguration {
    domain: String,
    payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagedSystemRoute {
    slug: String,
    version: String,
    enabled: bool,
}

async fn read_overlay_file() -> Result<ResourceOverlayFile> {
    let path = overlay_path();
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => {
            if raw.trim().is_empty() {
                return Ok(ResourceOverlayFile::default());
            }
            serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse overlay file `{}`", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(ResourceOverlayFile::default())
        }
        Err(error) => {
            Err(anyhow::Error::from(error).context(format!("failed to read `{}`", path.display())))
        }
    }
}

async fn write_overlay_file(overlay: &ResourceOverlayFile) -> Result<()> {
    let path = overlay_path();
    let tmp = path.with_extension("json.tmp");
    let serialized =
        serde_json::to_vec_pretty(overlay).context("failed to serialize resource overlay")?;
    tokio::fs::write(&tmp, &serialized)
        .await
        .with_context(|| format!("failed to write overlay tmp file `{}`", tmp.display()))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .with_context(|| format!("failed to commit overlay file `{}`", path.display()))?;
    Ok(())
}

fn parse_sealed_resource(name: &str, value: &serde_json::Value) -> Option<MeshResource> {
    let kind = value.get("type").and_then(|v| v.as_str())?.to_owned();
    let target = value.get("target").and_then(|v| v.as_str())?.to_owned();
    let allowed_methods = value
        .get("allowed_methods")
        .and_then(|v| v.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|entry| entry.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let version_constraint = value
        .get("version_constraint")
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    Some(MeshResource {
        name: name.to_owned(),
        kind,
        target,
        pending: false,
        allowed_methods,
        version_constraint,
    })
}

fn validate_resource_input(input: &MeshResourceInput) -> Result<MeshResource> {
    let name = input.name.trim();
    if name.is_empty() {
        anyhow::bail!("resource name must not be empty");
    }
    let kind = input.kind.trim().to_lowercase();
    if kind != "internal" && kind != "external" {
        anyhow::bail!("resource type must be either `internal` or `external`");
    }
    let target = input.target.trim();
    if target.is_empty() {
        anyhow::bail!("resource target must not be empty");
    }

    let mut allowed_methods: Vec<String> = Vec::new();
    let mut version_constraint: Option<String> = None;

    if kind == "external" {
        validate_external_target(name, target)?;
        if let Some(methods) = input.allowed_methods.clone() {
            for method in methods {
                let upper = method.trim().to_ascii_uppercase();
                if upper.is_empty() {
                    anyhow::bail!("external resource `{name}` must not declare empty HTTP methods");
                }
                if !upper
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '_')
                {
                    anyhow::bail!(
                        "external resource `{name}` has an invalid HTTP method `{upper}`"
                    );
                }
                if !allowed_methods.contains(&upper) {
                    allowed_methods.push(upper);
                }
            }
        }
    } else if let Some(constraint) = input.version_constraint.clone() {
        let trimmed = constraint.trim();
        if !trimmed.is_empty() {
            version_constraint = Some(trimmed.to_owned());
        }
    }

    Ok(MeshResource {
        name: name.to_owned(),
        kind,
        target: target.to_owned(),
        pending: true,
        allowed_methods,
        version_constraint,
    })
}

fn validate_external_target(name: &str, target: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(target.trim())
        .with_context(|| format!("external resource `{name}` target is not a valid URL"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("external resource `{name}` target must include a host"))?;

    if parsed.scheme() == "https" {
        return Ok(());
    }

    let host_lower = host.to_ascii_lowercase();
    let loopback_http = parsed.scheme() == "http"
        && (host_lower == "localhost"
            || host_lower
                .parse::<IpAddr>()
                .ok()
                .is_some_and(|ip| ip.is_loopback()));
    let cluster_local_http =
        parsed.scheme() == "http" && is_cluster_local_service_host(&host_lower);

    if !loopback_http && !cluster_local_http {
        anyhow::bail!(
            "external resource `{name}` target must use HTTPS unless it points at localhost or a cluster-local *.svc service"
        );
    }
    Ok(())
}

fn is_cluster_local_service_host(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    !normalized.is_empty()
        && normalized != "localhost"
        && (!normalized.contains('.')
            || normalized.ends_with(".svc")
            || normalized.ends_with(".svc.cluster.local"))
}

pub async fn read_resources() -> Result<Vec<MeshResource>> {
    let mut by_name: std::collections::BTreeMap<String, MeshResource> =
        std::collections::BTreeMap::new();

    let lockfile_path = workspace_root().join("integrity.lock");
    match tokio::fs::read_to_string(&lockfile_path).await {
        Ok(raw_lockfile) => {
            let manifest: IntegrityManifest = serde_json::from_str(&raw_lockfile)
                .context("failed to parse integrity.lock manifest")?;
            let sealed: SealedConfig = serde_json::from_str(&manifest.config_payload)
                .context("failed to parse sealed config payload")?;
            for (name, value) in sealed.resources {
                if let Some(resource) = parse_sealed_resource(&name, &value) {
                    by_name.insert(name, resource);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Lockfile missing on a fresh workspace — overlay alone is enough to render.
        }
        Err(error) => {
            return Err(anyhow::Error::from(error)
                .context(format!("failed to read {}", lockfile_path.display())));
        }
    }

    let overlay = read_overlay_file().await?;
    for entry in overlay.resources {
        by_name.insert(
            entry.name.clone(),
            MeshResource {
                pending: true,
                ..entry
            },
        );
    }

    Ok(by_name.into_values().collect())
}

pub async fn upsert_overlay_resource(input: MeshResourceInput) -> Result<MeshResource> {
    let resource = validate_resource_input(&input)?;
    let mut overlay = read_overlay_file().await?;
    if let Some(existing) = overlay
        .resources
        .iter_mut()
        .find(|entry| entry.name == resource.name)
    {
        *existing = resource.clone();
    } else {
        overlay.resources.push(resource.clone());
    }
    overlay.resources.sort_by(|a, b| a.name.cmp(&b.name));
    write_overlay_file(&overlay).await?;
    Ok(resource)
}

pub async fn stage_configuration_overlay(domain: &str, payload: serde_json::Value) -> Result<()> {
    let domain = domain.trim();
    if domain.is_empty() {
        anyhow::bail!("configuration domain must not be empty");
    }
    let mut overlay = read_overlay_file().await?;
    if let Some(existing) = overlay
        .configurations
        .iter_mut()
        .find(|entry| entry.domain == domain)
    {
        existing.payload = payload;
    } else {
        overlay.configurations.push(StagedConfiguration {
            domain: domain.to_owned(),
            payload,
        });
    }
    overlay
        .configurations
        .sort_by(|left, right| left.domain.cmp(&right.domain));
    write_overlay_file(&overlay).await?;
    Ok(())
}

/// Returns the list of system-faas routes staged in the local overlay.
pub async fn get_staged_system_routes() -> Result<Vec<(String, bool)>> {
    let overlay = read_overlay_file().await?;
    Ok(overlay
        .system_routes
        .into_iter()
        .map(|r| (r.slug, r.enabled))
        .collect())
}

/// Toggles a `system-faas-*` component in the overlay. `enabled = true` stages
/// it for addition on the next seal; `enabled = false` stages it for removal.
pub async fn toggle_system_route(slug: &str, version: &str, enabled: bool) -> Result<()> {
    let mut overlay = read_overlay_file().await?;
    if let Some(entry) = overlay.system_routes.iter_mut().find(|r| r.slug == slug) {
        entry.enabled = enabled;
        entry.version = version.to_owned();
    } else {
        overlay.system_routes.push(StagedSystemRoute {
            slug: slug.to_owned(),
            version: version.to_owned(),
            enabled,
        });
    }
    write_overlay_file(&overlay).await
}

/// Returns the staged (not-yet-sealed) payload for a given configuration domain,
/// or `None` if no overlay has been staged for that domain.
pub async fn get_staged_config(domain: &str) -> Result<Option<serde_json::Value>> {
    let overlay = read_overlay_file().await?;
    Ok(overlay
        .configurations
        .into_iter()
        .find(|entry| entry.domain == domain)
        .map(|entry| entry.payload))
}

/// Returns the **active** (sealed and applied) payload for a given configuration
/// domain, sourced from the live node's manifest when connected, or the local
/// `integrity.lock` file when offline. Returns `None` when the domain has no
/// active configuration.
pub async fn get_active_config(domain: &str) -> Result<Option<serde_json::Value>> {
    let payload_json: serde_json::Value = if current_connection().is_some() {
        match get_admin_json::<serde_json::Value>(ADMIN_MANIFEST_PATH).await {
            Ok(config) => config,
            Err(_) => {
                let raw = read_lockfile().await.unwrap_or_default();
                if raw.is_empty() {
                    return Ok(None);
                }
                let manifest: IntegrityManifest =
                    serde_json::from_str(&raw).context("failed to parse integrity.lock")?;
                serde_json::from_str(&manifest.config_payload)
                    .context("failed to parse config_payload")?
            }
        }
    } else {
        let raw = match read_lockfile().await {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };
        if raw.is_empty() {
            return Ok(None);
        }
        let manifest: IntegrityManifest =
            serde_json::from_str(&raw).context("failed to parse integrity.lock")?;
        serde_json::from_str(&manifest.config_payload).context("failed to parse config_payload")?
    };

    Ok(payload_json
        .get("ui_configurations")
        .and_then(|uc| uc.get(domain))
        .cloned())
}

pub async fn seal_overlay() -> Result<SealApplyOutcome> {
    let manifest = seal_overlay_manifest().await?;
    write_lockfile(&manifest).await?;
    let version = manifest_config_version(&manifest)?;
    Ok(SealApplyOutcome {
        success: true,
        message: format!("Overlay sealed into integrity.lock at config_version={version}."),
        config_version: version,
    })
}

pub async fn seal_and_apply_manifest() -> Result<SealApplyOutcome> {
    let manifest = seal_overlay_manifest().await?;
    write_lockfile(&manifest).await?;
    apply_manifest_to_active_node(&manifest).await
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleDependency {
    pub name: String,
    pub version_constraint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleConflict {
    pub name: String,
    pub bundled_version: String,
    pub cluster_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleApplyOutcome {
    pub success: bool,
    pub message: String,
    #[serde(default)]
    pub config_version: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts: Vec<BundleConflict>,
    #[serde(default)]
    pub requires_resolution: bool,
}

pub async fn bundle_and_apply_manifest(
    dependencies: Vec<BundleDependency>,
) -> Result<BundleApplyOutcome> {
    let manifest = seal_overlay_manifest().await?;
    let bytes = build_deployment_bundle(&manifest, &dependencies)?;
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_MANIFEST_BUNDLE_PATH)?)
        .bearer_auth(&config.token)
        .header(reqwest::header::CONTENT_TYPE, "application/gzip")
        .body(bytes)
        .send()
        .await
        .context("failed to POST deployment bundle")?;

    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read deployment bundle response body")?;

    if status == reqwest::StatusCode::PRECONDITION_REQUIRED {
        let outcome: BundleApplyOutcome = serde_json::from_slice(&body)
            .context("failed to decode deployment bundle 428 response payload")?;
        return Ok(BundleApplyOutcome {
            requires_resolution: true,
            ..outcome
        });
    }

    if !status.is_success() {
        anyhow::bail!(
            "deployment bundle apply failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    write_lockfile(&manifest).await?;
    let outcome: BundleApplyOutcome = serde_json::from_slice(&body)
        .context("failed to decode deployment bundle 200 response payload")?;
    Ok(outcome)
}

fn build_deployment_bundle(
    manifest: &IntegrityManifest,
    dependencies: &[BundleDependency],
) -> Result<Vec<u8>> {
    use flate2::{write::GzEncoder, Compression};
    use std::io::Write;
    use tar::{Builder, Header};

    let manifest_yaml = render_manifest_yaml(manifest, dependencies)?;
    let manifest_bytes = manifest_yaml.into_bytes();

    let buffer = Vec::new();
    let encoder = GzEncoder::new(buffer, Compression::default());
    let mut tar = Builder::new(encoder);
    let mut header = Header::new_gnu();
    header.set_path("manifest.yaml")?;
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append(&header, manifest_bytes.as_slice())
        .context("failed to append manifest.yaml to deployment bundle")?;

    let encoder = tar
        .into_inner()
        .context("failed to finalize deployment bundle tar stream")?;
    let mut buffer = encoder
        .finish()
        .context("failed to finalize deployment bundle gzip stream")?;
    buffer.flush().ok();
    Ok(buffer)
}

fn render_manifest_yaml(
    manifest: &IntegrityManifest,
    dependencies: &[BundleDependency],
) -> Result<String> {
    use std::fmt::Write as _;
    let mut yaml = String::new();
    writeln!(&mut yaml, "version: 1")?;
    // publicKey and signature are intentionally omitted — the receiving node
    // signs the manifest itself using its own HostIdentity key.
    writeln!(&mut yaml, "configPayload: |")?;
    for line in manifest.config_payload.lines() {
        writeln!(&mut yaml, "  {line}")?;
    }
    if !dependencies.is_empty() {
        writeln!(&mut yaml, "dependencies:")?;
        for dep in dependencies {
            let name = dep.name.replace([' ', '\n'], "_");
            writeln!(&mut yaml, "  {name}:")?;
            writeln!(
                &mut yaml,
                "    version: \"{}\"",
                dep.version_constraint.replace('"', "'")
            )?;
            if let Some(source) = dep.source.as_deref() {
                writeln!(&mut yaml, "    source: \"{}\"", source.replace('"', "'"))?;
            }
        }
    }
    Ok(yaml)
}

pub async fn get_node_public_key() -> Result<String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Response {
        public_key: String,
    }
    let response: Response = get_admin_json("/admin/identity/public-key").await?;
    Ok(response.public_key)
}

pub async fn apply_current_manifest() -> Result<SealApplyOutcome> {
    let raw = read_lockfile().await?;
    let manifest: IntegrityManifest =
        serde_json::from_str(&raw).context("failed to parse integrity.lock manifest")?;
    apply_manifest_to_active_node(&manifest).await
}

async fn seal_overlay_manifest() -> Result<IntegrityManifest> {
    let raw_lockfile = read_lockfile().await?;
    let current_manifest: IntegrityManifest =
        serde_json::from_str(&raw_lockfile).context("failed to parse integrity.lock manifest")?;
    let overlay = read_overlay_file().await?;
    let mut config: serde_json::Value = serde_json::from_str(&current_manifest.config_payload)
        .context("failed to parse sealed config payload")?;
    apply_overlay_to_config(&mut config, overlay)?;
    let payload =
        serde_json::to_string(&config).context("failed to serialize overlay config payload")?;
    sign_manifest_payload(payload).await
}

fn apply_overlay_to_config(
    config: &mut serde_json::Value,
    overlay: ResourceOverlayFile,
) -> Result<()> {
    let object = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("sealed config payload must be a JSON object"))?;
    let next_version = object
        .get("config_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    object.insert(
        "config_version".to_owned(),
        serde_json::Value::Number(next_version.into()),
    );

    if !overlay.resources.is_empty() {
        let resources = object
            .entry("resources".to_owned())
            .or_insert_with(|| serde_json::Value::Object(Default::default()));
        let resources = resources
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("sealed config resources must be a JSON object"))?;
        for resource in overlay.resources {
            let mut entry = serde_json::Map::new();
            entry.insert("type".to_owned(), serde_json::Value::String(resource.kind));
            entry.insert(
                "target".to_owned(),
                serde_json::Value::String(resource.target),
            );
            if !resource.allowed_methods.is_empty() {
                entry.insert(
                    "allowed_methods".to_owned(),
                    serde_json::Value::Array(
                        resource
                            .allowed_methods
                            .into_iter()
                            .map(serde_json::Value::String)
                            .collect(),
                    ),
                );
            }
            if let Some(version_constraint) = resource.version_constraint {
                entry.insert(
                    "version_constraint".to_owned(),
                    serde_json::Value::String(version_constraint),
                );
            }
            resources.insert(resource.name, serde_json::Value::Object(entry));
        }
    }

    if !overlay.configurations.is_empty() {
        let configs: BTreeMap<String, serde_json::Value> = overlay
            .configurations
            .into_iter()
            .map(|entry| (entry.domain, entry.payload))
            .collect();
        object.insert(
            "ui_configurations".to_owned(),
            serde_json::to_value(configs).context("failed to serialize staged configurations")?,
        );
    }

    // Merge system routes: enabled ones are added/updated; disabled ones are removed.
    let enabled: Vec<&StagedSystemRoute> =
        overlay.system_routes.iter().filter(|r| r.enabled).collect();
    if !overlay.system_routes.is_empty() {
        let routes = object
            .entry("routes".to_owned())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let arr = routes
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("routes must be an array"))?;

        // Remove any previously staged system routes that are now disabled.
        let disabled_slugs: std::collections::HashSet<&str> = overlay
            .system_routes
            .iter()
            .filter(|r| !r.enabled)
            .map(|r| r.slug.as_str())
            .collect();
        arr.retain(|route| {
            let name = route.get("name").and_then(|v| v.as_str()).unwrap_or("");
            !disabled_slugs.contains(name)
        });

        // Add enabled routes that are not already present.
        for sr in &enabled {
            let already_present = arr
                .iter()
                .any(|r| r.get("name").and_then(|v| v.as_str()) == Some(&sr.slug));
            if !already_present {
                let path = format!("/system/{}", sr.slug.replace("system-faas-", ""));
                arr.push(serde_json::json!({
                    "path": path,
                    "role": "system",
                    "name": sr.slug,
                    "version": sr.version,
                }));
            }
        }
    }

    Ok(())
}

async fn sign_manifest_payload(config_payload: String) -> Result<IntegrityManifest> {
    let signing_key = load_or_create_local_signing_key().await?;
    let signature = signing_key.sign(&Sha256::digest(config_payload.as_bytes()));
    Ok(IntegrityManifest {
        config_payload,
        public_key: hex::encode(signing_key.verifying_key().to_bytes()),
        signature: hex::encode(signature.to_bytes()),
    })
}

fn local_signing_key_path() -> PathBuf {
    workspace_root().join(".tachyon-local-ed25519")
}

async fn load_or_create_local_signing_key() -> Result<SigningKey> {
    let path = local_signing_key_path();
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let seed: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("local Ed25519 key has invalid length"))?;
            Ok(SigningKey::from_bytes(&seed))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let signing_key = SigningKey::generate(&mut OsRng);
            tokio::fs::write(&path, signing_key.to_bytes())
                .await
                .with_context(|| {
                    format!("failed to persist local Ed25519 key `{}`", path.display())
                })?;
            Ok(signing_key)
        }
        Err(error) => Err(anyhow::Error::from(error).context(format!(
            "failed to read local Ed25519 key `{}`",
            path.display()
        ))),
    }
}

async fn write_lockfile(manifest: &IntegrityManifest) -> Result<()> {
    let path = workspace_root().join("integrity.lock");
    let tmp = path.with_extension("lock.tmp");
    let bytes =
        serde_json::to_vec_pretty(manifest).context("failed to serialize sealed manifest")?;
    tokio::fs::write(&tmp, bytes)
        .await
        .with_context(|| format!("failed to write sealed manifest tmp `{}`", tmp.display()))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .with_context(|| format!("failed to replace `{}`", path.display()))?;
    Ok(())
}

fn manifest_config_version(manifest: &IntegrityManifest) -> Result<u64> {
    let payload: serde_json::Value = serde_json::from_str(&manifest.config_payload)
        .context("failed to parse sealed config payload")?;
    Ok(payload
        .get("config_version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0))
}

async fn apply_manifest_to_active_node(manifest: &IntegrityManifest) -> Result<SealApplyOutcome> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let body = serde_json::to_vec(manifest).context("failed to serialize admin manifest")?;
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_MANIFEST_PATH)?)
        .bearer_auth(&config.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .with_context(|| format!("failed to apply manifest to {}", config.url))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .context("failed to read admin manifest response body")?;
    if !status.is_success() {
        anyhow::bail!("manifest apply failed with {status}: {}", text.trim());
    }
    let version = manifest_config_version(manifest)?;
    Ok(SealApplyOutcome {
        success: true,
        message: text.trim().to_owned(),
        config_version: version,
    })
}

pub async fn dryrun_manifest(manifest: serde_json::Value) -> Result<DryRunManifestReport> {
    let config_payload = manifest
        .get("config_payload")
        .or_else(|| manifest.get("configPayload"))
        .and_then(serde_json::Value::as_str);
    let config = if let Some(payload) = config_payload {
        serde_json::from_str::<serde_json::Value>(payload)
            .context("failed to parse dry-run config payload")?
    } else {
        manifest
    };
    let object = config
        .as_object()
        .context("manifest dry-run payload must be a JSON object")?;

    let routes = object
        .get("routes")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let batch_targets = object
        .get("batch_targets")
        .or_else(|| object.get("batchTargets"))
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let resource_count = object
        .get("resources")
        .and_then(serde_json::Value::as_object)
        .map(|resources| resources.len())
        .unwrap_or(0);
    let config_version = object
        .get("config_version")
        .or_else(|| object.get("configVersion"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let mut warnings = Vec::new();
    if routes.is_empty() && batch_targets.is_empty() && resource_count == 0 {
        warnings.push("manifest contains no routes, batch targets, or resources".to_owned());
    }
    for (index, route) in routes.iter().enumerate() {
        if route
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            warnings.push(format!("route[{index}] is missing a path"));
        }
        if route.get("targets").is_none() && route.get("target").is_none() {
            warnings.push(format!(
                "route[{index}] is missing a target or targets list"
            ));
        }
    }
    for (index, batch_target) in batch_targets.iter().enumerate() {
        if batch_target
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        {
            warnings.push(format!("batchTargets[{index}] is missing a name"));
        }
    }

    Ok(DryRunManifestReport {
        valid: warnings.is_empty(),
        config_version,
        route_count: routes.len(),
        batch_target_count: batch_targets.len(),
        resource_count,
        warnings,
    })
}

pub async fn get_metrics() -> Result<RuntimeMetrics> {
    if current_connection().is_some() {
        return get_admin_json(ADMIN_METRICS_PATH).await;
    }

    let graph = get_mesh_graph().await?;
    Ok(RuntimeMetrics {
        source: graph.source,
        error_rate: 0.0,
        p50_latency_ms: 0.0,
        p99_latency_ms: 0.0,
        queue_depth: graph.batch_targets.len() as u64,
        vram_utilization_pct: 0,
        ram_offload_active: false,
        scope_denial_total: 0,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanaryStatusEntry {
    pub route_path: String,
    pub current_version: String,
    pub next_version: String,
    pub weight_pct: u32,
    pub phase: String,
    pub next_req_count: u64,
    pub next_err_count: u64,
}

pub async fn fetch_canary_status() -> Result<Vec<CanaryStatusEntry>> {
    if current_connection().is_none() {
        return Ok(Vec::new());
    }
    get_admin_json(ADMIN_CANARY_PATH).await
}

/// Fetch the JSON Schema document that describes the `IntegrityConfig` manifest
/// format accepted by `POST /admin/manifest`. Returns `None` when not connected
/// or when the host does not expose the schema endpoint.
pub async fn get_manifest_schema() -> Result<serde_json::Value> {
    if current_connection().is_none() {
        return Ok(serde_json::json!({ "type": "object" }));
    }
    get_admin_json(ADMIN_SCHEMA_MANIFEST_PATH).await
}

/// Fetch and parse the current live manifest's `config_payload` from the node.
/// Returns the config as a `serde_json::Value` ready for modification.
/// Errors if not connected or if the node returns an error.
pub async fn get_manifest_config() -> Result<serde_json::Value> {
    get_admin_json::<serde_json::Value>(ADMIN_MANIFEST_PATH)
        .await
        .context("failed to fetch live manifest from node")
}

/// Sign a modified manifest config and POST it to the active node.
/// The `config` value is the parsed `config_payload` (same shape returned by
/// `get_manifest_config`). Increments `config_version` before signing.
pub async fn apply_manifest_config(mut config: serde_json::Value) -> Result<SealApplyOutcome> {
    if let Some(obj) = config.as_object_mut() {
        let next_version = obj
            .get("config_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .saturating_add(1);
        obj.insert(
            "config_version".to_owned(),
            serde_json::Value::Number(next_version.into()),
        );
    }
    let payload =
        serde_json::to_string(&config).context("failed to serialize manifest config_payload")?;
    let manifest = sign_manifest_payload(payload).await?;
    apply_manifest_to_active_node(&manifest).await
}

pub async fn abort_canary_rollout(route_path: &str) -> Result<()> {
    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }
    let payload = serde_json::json!({ "routePath": route_path });
    post_admin_json::<serde_json::Value, serde_json::Value>(ADMIN_CANARY_PATH, &payload).await?;
    Ok(())
}

// ── KV-Partition V2 admin operations ─────────────────────────────────────────

/// Read a value from the KV-Partition V2 store.
pub async fn kv_get(namespace: &str, key: &str) -> Result<Option<Vec<u8>>> {
    if current_connection().is_none() {
        return Ok(None);
    }
    let path = format!("{ADMIN_KV_PATH}/{namespace}/{key}");
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let url = build_endpoint_url(&config.url, &path)?;
    let response = client.get(url).bearer_auth(&config.token).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        anyhow::bail!("kv_get failed: HTTP {}", response.status());
    }
    Ok(Some(response.bytes().await?.to_vec()))
}

/// Write a value to the KV-Partition V2 store.
pub async fn kv_put(namespace: &str, key: &str, value: &[u8]) -> Result<()> {
    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }
    let path = format!("{ADMIN_KV_PATH}/{namespace}/{key}");
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let url = build_endpoint_url(&config.url, &path)?;
    let response = client
        .put(url)
        .bearer_auth(&config.token)
        .body(value.to_vec())
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("kv_put failed: HTTP {}", response.status());
    }
    Ok(())
}

/// Delete a key from the KV-Partition V2 store.
pub async fn kv_delete(namespace: &str, key: &str) -> Result<()> {
    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }
    let path = format!("{ADMIN_KV_PATH}/{namespace}/{key}");
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let url = build_endpoint_url(&config.url, &path)?;
    let response = client.delete(url).bearer_auth(&config.token).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("kv_delete failed: HTTP {}", response.status());
    }
    Ok(())
}

// ── WASM function lifecycle ───────────────────────────────────────────────────

/// List deployed functions (routes) by reading the live mesh graph.
pub async fn list_functions() -> Result<Vec<MeshRouteSummary>> {
    if current_connection().is_none() {
        return Ok(Vec::new());
    }
    let graph = get_mesh_graph().await?;
    Ok(graph.routes)
}

/// Deploy a pre-compiled WASM artifact to the mesh.
/// Uploads the bytes as a named asset, then stages a workload configuration
/// overlay that the operator can seal and apply via `tachyon_seal_overlay`.
pub async fn deploy_function(
    function_name: &str,
    wasm_bytes: Vec<u8>,
    memory_mb: u64,
    gpu_vram_mb: u64,
) -> Result<serde_json::Value> {
    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }
    let asset_path = push_asset_bytes(function_name, &wasm_bytes).await?;
    let payload = serde_json::json!({
        "engine": "wasmtime",
        "function_name": function_name,
        "artifact_path": asset_path,
        "memory_mb": memory_mb,
        "gpu_vram_mb": gpu_vram_mb,
    });
    stage_configuration_overlay("workloads", payload).await?;
    Ok(serde_json::json!({
        "function_name": function_name,
        "asset_path": asset_path,
        "status": "staged",
        "note": "Run tachyon_seal_overlay to activate the deployment"
    }))
}

/// Remove a deployed function from the overlay.
pub async fn delete_function(function_name: &str) -> Result<()> {
    remove_overlay_resource(function_name).await
}

/// Fetch recent logs for a specific deployed function.
/// Filters the global audit log by target/route containing `function_name`.
pub async fn function_logs(function_name: &str, lines: usize) -> Result<Vec<LogLine>> {
    if current_connection().is_none() {
        return Ok(Vec::new());
    }
    let limit = lines.clamp(1, 1_000);
    let params = [
        ("lines", (limit * 4).to_string()),
        ("target", function_name.to_owned()),
    ];
    let all: Vec<LogLine> = get_admin_json_with_query(ADMIN_LOGS_PATH, &params).await?;
    Ok(all
        .into_iter()
        .filter(|l| l.target.contains(function_name) || l.message.contains(function_name))
        .take(limit)
        .collect())
}

/// Update the live traffic-split weight for a canary rollout on `route_path`.
/// If `weight_pct` is 0 the rollout is aborted via the abort endpoint;
/// otherwise the weight is updated directly via `PATCH /admin/canary`.
pub async fn set_canary_split(route_path: &str, weight_pct: u8) -> Result<()> {
    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }
    if weight_pct == 0 {
        return abort_canary_rollout(route_path).await;
    }
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let url = build_endpoint_url(&config.url, ADMIN_CANARY_PATH)?;
    let payload = serde_json::json!({
        "routePath": route_path,
        "weightPct": weight_pct,
    });
    let response = client
        .patch(url)
        .bearer_auth(&config.token)
        .json(&payload)
        .send()
        .await?;
    if !response.status().is_success() {
        anyhow::bail!("set_canary_split failed: HTTP {}", response.status());
    }
    Ok(())
}

pub async fn tail_logs(lines: usize) -> Result<Vec<LogLine>> {
    if current_connection().is_none() {
        return Ok(Vec::new());
    }

    let limit = lines.clamp(1, 1_000);
    get_admin_json_with_query(ADMIN_LOGS_PATH, &[("lines", limit.to_string())]).await
}

pub async fn get_shadow_diffs() -> Result<Vec<ShadowDiff>> {
    if current_connection().is_none() {
        return Ok(Vec::new());
    }

    get_admin_json(ADMIN_SHADOW_DIFFS_PATH).await
}

pub async fn run_chaos_scenario(request: ChaosScenarioRequest) -> Result<ChaosScenarioOutcome> {
    if request.scenario.trim().is_empty() {
        anyhow::bail!("chaos scenario must not be empty");
    }
    if request.duration_seconds.unwrap_or(0) > 3_600 {
        anyhow::bail!("chaos scenario duration must be less than or equal to 3600 seconds");
    }

    post_admin_json(ADMIN_CHAOS_SCENARIOS_PATH, &request).await
}

// ── S3 FaaS volume management ─────────────────────────────────────────────────

/// One S3 volume entry as returned by `list_s3_volumes`.
#[derive(Debug, Serialize, Deserialize)]
pub struct S3VolumeEntry {
    pub s3_url: String,
    pub guest_path: String,
    pub readonly: bool,
}

/// Read the live manifest and return S3 volumes configured for `route_path`.
pub async fn list_s3_volumes(route_path: &str) -> Result<Vec<S3VolumeEntry>> {
    let config = load_live_config_payload().await?;
    let routes = config
        .get("routes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    for route in &routes {
        if route.get("path").and_then(|v| v.as_str()) == Some(route_path) {
            let volumes = route
                .get("volumes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let s3: Vec<S3VolumeEntry> = volumes
                .into_iter()
                .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("s3"))
                .map(|v| S3VolumeEntry {
                    s3_url: v
                        .get("host_path")
                        .and_then(|h| h.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    guest_path: v
                        .get("guest_path")
                        .and_then(|g| g.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    readonly: v.get("readonly").and_then(|r| r.as_bool()).unwrap_or(false),
                })
                .collect();
            return Ok(s3);
        }
    }
    anyhow::bail!("route `{route_path}` not found in sealed manifest")
}

/// Add an S3 volume to a route in the live sealed manifest.
/// The `s3_url` must be in the form `s3://bucket/prefix`.
pub async fn attach_s3_volume(
    route_path: &str,
    s3_url: &str,
    guest_path: &str,
    readonly: bool,
) -> Result<S3VolumeEntry> {
    if !s3_url.starts_with("s3://") {
        anyhow::bail!("invalid S3 URL `{s3_url}`: must start with s3://");
    }

    let mut config = load_live_config_payload().await?;
    let routes = config
        .get_mut("routes")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("manifest has no routes array"))?;

    let route = routes
        .iter_mut()
        .find(|r| r.get("path").and_then(|v| v.as_str()) == Some(route_path))
        .ok_or_else(|| anyhow::anyhow!("route `{route_path}` not found in sealed manifest"))?;

    let volumes = route.get_mut("volumes").and_then(|v| v.as_array_mut());

    let new_volume = serde_json::json!({
        "type": "s3",
        "host_path": s3_url,
        "guest_path": guest_path,
        "readonly": readonly,
    });

    if let Some(arr) = volumes {
        if arr.iter().any(|v| {
            v.get("type").and_then(|t| t.as_str()) == Some("s3")
                && v.get("guest_path").and_then(|g| g.as_str()) == Some(guest_path)
        }) {
            anyhow::bail!(
                "route `{route_path}` already has an S3 volume at guest path `{guest_path}`"
            );
        }
        arr.push(new_volume);
    } else {
        route["volumes"] = serde_json::json!([new_volume]);
    }

    patch_and_apply_manifest(config).await?;

    Ok(S3VolumeEntry {
        s3_url: s3_url.to_owned(),
        guest_path: guest_path.to_owned(),
        readonly,
    })
}

/// Remove an S3 volume (identified by `guest_path`) from a route in the live manifest.
pub async fn detach_s3_volume(route_path: &str, guest_path: &str) -> Result<()> {
    let mut config = load_live_config_payload().await?;
    let routes = config
        .get_mut("routes")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("manifest has no routes array"))?;

    let route = routes
        .iter_mut()
        .find(|r| r.get("path").and_then(|v| v.as_str()) == Some(route_path))
        .ok_or_else(|| anyhow::anyhow!("route `{route_path}` not found in sealed manifest"))?;

    let volumes = route
        .get_mut("volumes")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| anyhow::anyhow!("route `{route_path}` has no volumes configured"))?;

    let before = volumes.len();
    volumes.retain(|v| {
        !(v.get("type").and_then(|t| t.as_str()) == Some("s3")
            && v.get("guest_path").and_then(|g| g.as_str()) == Some(guest_path))
    });
    if volumes.len() == before {
        anyhow::bail!("route `{route_path}` has no S3 volume at guest path `{guest_path}`");
    }

    patch_and_apply_manifest(config).await
}

/// Fetch the live IntegrityConfig payload as a mutable JSON Value.
/// Falls back to the local integrity.lock file when not connected.
async fn load_live_config_payload() -> Result<serde_json::Value> {
    if current_connection().is_some() {
        if let Ok(config) = get_admin_json::<serde_json::Value>(ADMIN_MANIFEST_PATH).await {
            return Ok(config);
        }
    }

    // Fall back to local integrity.lock (wrapper format: { config_payload, public_key, signature })
    #[derive(Deserialize)]
    struct IntegrityManifestFile {
        config_payload: String,
    }
    let raw = read_lockfile().await?;
    let manifest: IntegrityManifestFile =
        serde_json::from_str(&raw).context("failed to parse integrity.lock")?;
    serde_json::from_str(&manifest.config_payload)
        .context("failed to parse integrity.lock config_payload")
}

/// Serialize `config` back to a string, re-sign, write the lockfile, and POST to /admin/manifest.
async fn patch_and_apply_manifest(mut config: serde_json::Value) -> Result<()> {
    if let Some(obj) = config.as_object_mut() {
        let next = obj
            .get("config_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .saturating_add(1);
        obj.insert(
            "config_version".to_owned(),
            serde_json::Value::Number(next.into()),
        );
    }
    let payload =
        serde_json::to_string(&config).context("failed to serialize patched config payload")?;
    let manifest = sign_manifest_payload(payload).await?;
    write_lockfile(&manifest).await?;
    apply_manifest_to_active_node(&manifest).await?;
    Ok(())
}

// ── Volume backup management ──────────────────────────────────────────────────

/// Metadata for a single volume backup snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSnapshot {
    pub snapshot_id: String,
    pub route_path: String,
    pub guest_path: String,
    pub timestamp_ms: u64,
    pub object_count: usize,
}

/// Trigger a backup of a route volume and return the snapshot metadata.
pub async fn backup_volume(route_path: &str, guest_path: &str) -> Result<BackupSnapshot> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let body = serde_json::json!({ "route_path": route_path, "guest_path": guest_path });
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_VOLUMES_BACKUP_PATH)?)
        .bearer_auth(&config.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&body)?)
        .send()
        .await
        .context("failed to call /admin/volumes/backup")?;
    decode_admin_response(response, ADMIN_VOLUMES_BACKUP_PATH).await
}

/// Restore a route volume from a previously created snapshot.
pub async fn restore_volume(route_path: &str, guest_path: &str, snapshot_id: &str) -> Result<()> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let body = serde_json::json!({
        "route_path": route_path,
        "guest_path": guest_path,
        "snapshot_id": snapshot_id,
    });
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_VOLUMES_RESTORE_PATH)?)
        .bearer_auth(&config.token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::to_vec(&body)?)
        .send()
        .await
        .context("failed to call /admin/volumes/restore")?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let text = response.text().await.unwrap_or_default();
    anyhow::bail!("restore failed with {status}: {}", text.trim())
}

/// List available snapshots for a route volume, newest first.
pub async fn list_volume_backups(
    route_path: &str,
    guest_path: &str,
) -> Result<Vec<BackupSnapshot>> {
    get_admin_json_with_query::<Vec<BackupSnapshot>>(
        ADMIN_VOLUMES_BACKUPS_PATH,
        &[
            ("route_path", route_path.to_owned()),
            ("guest_path", guest_path.to_owned()),
        ],
    )
    .await
}

// ── Concurrency policy recommendation ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyRequirements {
    #[serde(default)]
    pub writes_shared_state: bool,
    #[serde(default)]
    pub requires_ordering: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyConfig {
    pub mode: String,
    pub on_conflict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_ttl_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyConfig {
    pub read_mode: String,
    pub write_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinationConfig {
    pub mode: String,
    pub write_isolation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConcurrencyRecommendation {
    pub concurrency: ConcurrencyConfig,
    pub consistency: ConsistencyConfig,
    pub coordination: CoordinationConfig,
    pub rationale: String,
    /// `low` | `medium` | `high`
    pub risk_level: String,
    pub trade_offs: Vec<String>,
}

/// Map a usage pattern + requirements to a recommended concurrency, consistency
/// and coordination configuration. Implemented as a pure local decision table —
/// no HTTP call, no LLM. The UI and MCP layers both use this so a draft policy
/// is always available even when the core-host is unreachable.
pub fn recommend_concurrency_policy(
    pattern: &str,
    requirements: &ConcurrencyRequirements,
) -> ConcurrencyRecommendation {
    let writes_shared = requirements.writes_shared_state;
    let ordered = requirements.requires_ordering;
    let latency_sensitive = requirements
        .max_latency_ms
        .map(|ms| ms <= 200)
        .unwrap_or(false);

    let (concurrency, consistency, coordination, rationale, risk, trade_offs) = match pattern {
        "batch" => (
            ("node-singleton", "queue", None),
            ("snapshot", "last_write_wins"),
            ("mesh_leader", "none"),
            "Batch jobs typically run on a leader node with one invocation at a time per node.",
            "low",
            vec!["Higher latency when bursts queue", "Predictable resource usage"],
        ),
        "interactive" => (
            ("unrestricted", "queue", None),
            ("snapshot", "last_write_wins"),
            ("per_node", "none"),
            "Interactive workloads optimize for latency and accept best-effort consistency.",
            "low",
            vec!["No write-conflict protection — only safe for read-mostly or per-tenant state"],
        ),
        "stateful" => (
            ("mesh-singleton", "queue", Some(30_000u64)),
            ("live", "pessimistic_lock"),
            ("mesh_leader", "drain"),
            "Stateful services require strict serialization and consistent snapshots.",
            "low",
            vec![
                "Added latency under load (serialized via distributed lock)",
                "Backups pause new invocations during snapshot",
            ],
        ),
        "etl" => (
            ("mesh-leader", "reject", None),
            ("snapshot", "optimistic_etag"),
            ("mesh_leader", "none"),
            "ETL pipelines run on a single elected node; concurrent writers fail fast on ETag conflict.",
            "medium",
            vec![
                "Optimistic conflict failures require client-side retry",
                "Non-leader nodes return 503 with a leader hint",
            ],
        ),
        "scheduler" => (
            ("mesh-leader", "reject", None),
            ("snapshot", "last_write_wins"),
            ("mesh_leader", "none"),
            "Scheduler-style jobs MUST run on a single node to avoid duplicate work.",
            "low",
            vec!["Non-leader nodes reject invocations with a leader hint"],
        ),
        _ => {
            // Unknown pattern: derive from requirements.
            if writes_shared && ordered {
                (
                    ("mesh-singleton", "queue", Some(30_000u64)),
                    ("live", "pessimistic_lock"),
                    ("mesh_leader", "drain"),
                    "Unknown pattern with shared writable state and ordering — defaulting to stateful mode.",
                    "low",
                    vec!["Higher latency", "Most conservative default"],
                )
            } else if writes_shared {
                (
                    ("mesh-leader", "reject", None),
                    ("snapshot", "optimistic_etag"),
                    ("mesh_leader", "none"),
                    "Unknown pattern with shared writable state — defaulting to optimistic concurrency.",
                    "medium",
                    vec!["ETag conflicts will surface as 412 errors"],
                )
            } else if latency_sensitive {
                (
                    ("unrestricted", "queue", None),
                    ("snapshot", "last_write_wins"),
                    ("per_node", "none"),
                    "Latency-sensitive without shared writes — using interactive defaults.",
                    "low",
                    vec!["No write-conflict protection (none needed)"],
                )
            } else {
                (
                    ("unrestricted", "queue", None),
                    ("snapshot", "last_write_wins"),
                    ("per_node", "none"),
                    "No pattern hint provided — returning default permissive policy.",
                    "low",
                    vec!["Tighten when requirements are known"],
                )
            }
        }
    };

    ConcurrencyRecommendation {
        concurrency: ConcurrencyConfig {
            mode: concurrency.0.to_owned(),
            on_conflict: concurrency.1.to_owned(),
            lock_ttl_ms: concurrency.2,
        },
        consistency: ConsistencyConfig {
            read_mode: consistency.0.to_owned(),
            write_mode: consistency.1.to_owned(),
        },
        coordination: CoordinationConfig {
            mode: coordination.0.to_owned(),
            write_isolation: coordination.1.to_owned(),
        },
        rationale: rationale.to_owned(),
        risk_level: risk.to_owned(),
        trade_offs: trade_offs.into_iter().map(str::to_owned).collect(),
    }
}

pub async fn list_available_ai_models() -> Result<Vec<AvailableAiModel>> {
    let response: OpenAiModelListResponse = get_admin_json("/v1/models").await?;
    Ok(response
        .data
        .into_iter()
        .map(|model| available_model_from_openai_id(&model.id))
        .collect())
}

fn available_model_from_openai_id(id: &str) -> AvailableAiModel {
    let (engine, alias) = id.split_once('/').unwrap_or(("unknown", id));
    AvailableAiModel {
        id: id.to_owned(),
        alias: alias.to_owned(),
        engine: engine.to_owned(),
    }
}

pub async fn remove_overlay_resource(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("resource name must not be empty");
    }
    let mut overlay = read_overlay_file().await?;
    let original_len = overlay.resources.len();
    overlay.resources.retain(|entry| entry.name != trimmed);
    if overlay.resources.len() == original_len {
        anyhow::bail!(
            "resource `{trimmed}` is not in the workspace overlay; sealed entries require a CLI re-seal of integrity.lock to remove"
        );
    }
    write_overlay_file(&overlay).await?;
    Ok(())
}

pub async fn set_connection(
    url: String,
    token: String,
    cert: Option<Vec<u8>>,
) -> Result<(), String> {
    let config = InstanceConfig {
        url: url.trim().to_owned(),
        token: token.trim().to_owned(),
        mtls_cert: cert,
        mtls_key: None,
    };
    validate_connection_config(&config).map_err(|error| error.to_string())?;
    fetch_remote_status(&config)
        .await
        .map_err(|error| error.to_string())?;

    let mut state = connection_state()
        .write()
        .expect("connection state should not be poisoned");
    *state = Some(config);
    Ok(())
}

pub async fn authn_login(
    url: &str,
    username: &str,
    password: &str,
    cert: Option<Vec<u8>>,
) -> Result<AuthLoginResponse> {
    let normalized_username = normalize_operator_name(username)?;
    let config = public_connection_config(url, cert);
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, AUTH_LOGIN_STAGE_PATH)?)
        .json(&StageLoginRequest {
            username: &normalized_username,
            password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read login staging response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "login staging failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let staged: StagedLoginResponse =
        serde_json::from_slice(&body).context("failed to decode login staging response payload")?;

    Ok(AuthLoginResponse {
        username: staged.username,
        endpoint: config.url,
        requires_mfa: true,
        session_id: Some(staged.session_id),
    })
}

pub async fn finalize_login(
    url: &str,
    session_id: &str,
    totp_code: &str,
    cert: Option<Vec<u8>>,
) -> Result<AuthLoginResponse> {
    let config = public_connection_config(url, cert.clone());
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, AUTH_LOGIN_FINALIZE_PATH)?)
        .json(&FinalizeLoginRequest {
            session_id: session_id.trim(),
            totp_code: totp_code.trim(),
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read login finalization response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "login finalization failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: FinalizeEnrollmentResponse = serde_json::from_slice(&body)
        .context("failed to decode login finalization response payload")?;
    set_connection(config.url.clone(), payload.token, cert)
        .await
        .map_err(anyhow::Error::msg)?;

    let mut operator = operator_state()
        .write()
        .expect("operator state should not be poisoned");
    *operator = Some(payload.username.clone());

    Ok(AuthLoginResponse {
        username: payload.username,
        endpoint: config.url,
        requires_mfa: false,
        session_id: None,
    })
}

pub async fn verify_session_totp(totp_code: &str) -> Result<MfaSessionToken> {
    let trimmed = totp_code.trim();
    if trimmed.len() != 6 || !trimmed.chars().all(|digit| digit.is_ascii_digit()) {
        anyhow::bail!("MFA code must contain exactly 6 digits");
    }

    post_admin_json(
        ADMIN_STEP_UP_PATH,
        &StepUpSessionRequest { totp_code: trimmed },
    )
    .await
}

pub async fn validate_registration_token(
    url: &str,
    token: &str,
    cert: Option<Vec<u8>>,
) -> Result<RegistrationTokenClaims> {
    let config = public_connection_config(url, cert);
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, AUTH_SIGNUP_VALIDATE_PATH)?)
        .json(&ValidateRegistrationTokenRequest {
            token: token.trim(),
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read registration-token response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "registration token validation failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    serde_json::from_slice(&body).context("failed to decode registration-token response payload")
}

pub async fn stage_signup(
    url: &str,
    token: &str,
    first_name: &str,
    last_name: &str,
    username: &str,
    password: &str,
    cert: Option<Vec<u8>>,
) -> Result<StagedSignupSession> {
    let config = public_connection_config(url, cert);
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, AUTH_SIGNUP_STAGE_PATH)?)
        .json(&StageSignupRequest {
            token: token.trim(),
            first_name: first_name.trim(),
            last_name: last_name.trim(),
            username: username.trim(),
            password,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read signup staging response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "signup staging failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    serde_json::from_slice(&body).context("failed to decode signup staging response payload")
}

pub async fn finalize_enrollment(
    url: &str,
    session_id: &str,
    totp_code: &str,
    cert: Option<Vec<u8>>,
) -> Result<AuthLoginResponse> {
    let config = public_connection_config(url, cert.clone());
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, AUTH_SIGNUP_FINALIZE_PATH)?)
        .json(&FinalizeEnrollmentRequest {
            session_id: session_id.trim(),
            totp_code: totp_code.trim(),
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read enrollment-finalize response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "enrollment finalization failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: FinalizeEnrollmentResponse = serde_json::from_slice(&body)
        .context("failed to decode enrollment-finalize response payload")?;
    set_connection(config.url.clone(), payload.token, cert)
        .await
        .map_err(anyhow::Error::msg)?;

    let mut operator = operator_state()
        .write()
        .expect("operator state should not be poisoned");
    *operator = Some(payload.username.clone());

    Ok(AuthLoginResponse {
        username: payload.username,
        endpoint: config.url,
        requires_mfa: false,
        session_id: None,
    })
}

pub async fn iam_list_users() -> Result<Vec<IamUserSummary>> {
    get_admin_json(ADMIN_IAM_USERS_PATH).await
}

pub async fn iam_update_user(username: &str, update: &IamUserUpdate) -> Result<IamUserSummary> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        anyhow::bail!("username must not be empty");
    }
    patch_admin_json(&format!("{ADMIN_IAM_USERS_PATH}/{trimmed}"), update).await
}

pub async fn iam_delete_user(username: &str) -> Result<()> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        anyhow::bail!("username must not be empty");
    }
    delete_admin_path(&format!("{ADMIN_IAM_USERS_PATH}/{trimmed}")).await
}

pub async fn iam_list_groups() -> Result<Vec<IamGroupSummary>> {
    get_admin_json(ADMIN_IAM_GROUPS_PATH).await
}

pub async fn iam_upsert_group(input: &IamGroupInput) -> Result<IamGroupSummary> {
    if input.name.trim().is_empty() {
        anyhow::bail!("group name must not be empty");
    }
    post_admin_json(ADMIN_IAM_GROUPS_PATH, input).await
}

pub async fn iam_delete_group(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("group name must not be empty");
    }
    delete_admin_path(&format!("{ADMIN_IAM_GROUPS_PATH}/{trimmed}")).await
}

pub async fn fetch_user_audit_log(user: Option<&str>, lines: usize) -> Result<Vec<IamAuditEntry>> {
    let limit = lines.clamp(1, 500);
    let mut params: Vec<(&str, String)> = vec![("lines", limit.to_string())];
    if let Some(value) = user {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            params.push(("user", trimmed.to_owned()));
        }
    }
    get_admin_json_with_query(ADMIN_LOGS_PATH, &params).await
}

pub async fn iam_regen_mfa(username: &str) -> Result<Vec<String>> {
    let active = current_operator_name().unwrap_or_else(|| "admin".to_owned());
    if username.trim() != active {
        anyhow::bail!("only the active administrative session can rotate its recovery bundle");
    }

    regenerate_account_security().await
}

pub async fn generate_recovery_codes(username: &str) -> Result<Vec<String>> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_RECOVERY_CODES_PATH)?)
        .bearer_auth(&config.token)
        .json(&RecoveryCodeRequest { username })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read recovery-code response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "recovery-code request failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: RecoveryCodeResponse =
        serde_json::from_slice(&body).context("failed to decode recovery-code response payload")?;
    Ok(payload.codes)
}

pub async fn regenerate_account_security() -> Result<Vec<String>> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(
            &config.url,
            ADMIN_ACCOUNT_SECURITY_PATH,
        )?)
        .bearer_auth(&config.token)
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read account-security response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "account security regeneration failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: RecoveryCodeResponse = serde_json::from_slice(&body)
        .context("failed to decode account-security response payload")?;
    Ok(payload.codes)
}

pub async fn generate_pat(name: &str, scopes: &[String], ttl_days: u32) -> Result<String> {
    if name.trim().is_empty() {
        anyhow::bail!("PAT name must not be empty");
    }
    if scopes.is_empty() {
        anyhow::bail!("PAT scopes must not be empty");
    }

    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_PAT_PATH)?)
        .bearer_auth(&config.token)
        .json(&IssuePatRequest {
            name: name.trim(),
            scopes,
            ttl_days,
        })
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read PAT response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "PAT issuance failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: IssuePatResponse =
        serde_json::from_slice(&body).context("failed to decode PAT response payload")?;
    Ok(payload.token)
}

pub async fn start_enrollment_invite(node_public_key: &str) -> Result<EnrollmentInvite> {
    let trimmed = node_public_key.trim();
    if trimmed.is_empty() {
        anyhow::bail!("node public key must not be empty");
    }

    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(
            &config.url,
            ADMIN_ENROLLMENT_START_PATH,
        )?)
        .bearer_auth(&config.token)
        .json(&AdminEnrollmentStartRequest {
            node_public_key: trimmed,
        })
        .send()
        .await
        .with_context(|| format!("failed to start enrollment invite at {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read enrollment invite response body")?;
    if !status.is_success() {
        anyhow::bail!(
            "enrollment invite failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: AdminEnrollmentStartResponse =
        serde_json::from_slice(&body).context("failed to decode enrollment invite payload")?;
    let qr_payload = format!(
        "tachyon://enroll?endpoint={}&session_id={}&pin={}",
        config.url, payload.session_id, payload.pin
    );

    Ok(EnrollmentInvite {
        session_id: payload.session_id,
        invite_token: payload.pin,
        qr_payload,
    })
}

pub async fn push_asset(file_path: &str) -> Result<String> {
    let bytes = tokio::fs::read(file_path)
        .await
        .with_context(|| format!("failed to read asset payload from `{file_path}`"))?;
    push_asset_bytes(file_path, &bytes).await
}

pub async fn push_asset_bytes(path: &str, bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() {
        anyhow::bail!("asset payload `{path}` must not be empty");
    }

    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let expected_hash = sha256_hash(bytes);
    let response = client
        .post(build_endpoint_url(&config.url, ADMIN_ASSET_UPLOAD_PATH)?)
        .bearer_auth(&config.token)
        .header(EXPECTED_HASH_HEADER, &expected_hash)
        .header(reqwest::header::CONTENT_TYPE, "application/wasm")
        .body(bytes.to_vec())
        .send()
        .await
        .with_context(|| format!("failed to upload asset `{path}` to {}", config.url))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .context("failed to read asset-upload response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "asset upload failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    let payload: AssetUploadResponse =
        serde_json::from_slice(&body).context("failed to decode asset-upload response payload")?;
    Ok(payload.asset_uri)
}

/// A single WASM module uploaded during a package import.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedModule {
    pub name: String,
    pub asset_uri: String,
}

/// Result returned by [`import_faas_package`] / [`import_faas_package_bytes`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPackageResult {
    pub imported_modules: Vec<ImportedModule>,
    pub skipped_modules: Vec<String>,
    pub routes_added: usize,
}

/// Read a `.tar.gz` FaaS package from `package_path`, upload every `.wasm`
/// inside it as an asset, then register the routes from `manifest.json`.
pub async fn import_faas_package(package_path: &str) -> Result<ImportPackageResult> {
    let bytes = tokio::fs::read(package_path)
        .await
        .with_context(|| format!("failed to read package `{package_path}`"))?;
    import_faas_package_bytes(&bytes).await
}

/// Same as [`import_faas_package`] but accepts the archive bytes directly.
pub async fn import_faas_package_bytes(data: &[u8]) -> Result<ImportPackageResult> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tar::Archive;

    if current_connection().is_none() {
        return Err(anyhow::anyhow!("not connected to a node"));
    }

    let gz = GzDecoder::new(data);
    let mut archive = Archive::new(gz);

    let mut wasm_files: std::collections::HashMap<String, Vec<u8>> =
        std::collections::HashMap::new();
    let mut manifest_bytes: Option<Vec<u8>> = None;

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if filename == "manifest.json" {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            manifest_bytes = Some(buf);
        } else if let Some(stem) = filename.strip_suffix(".wasm") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            wasm_files.insert(stem.to_owned(), buf);
        }
    }

    let manifest_bytes =
        manifest_bytes.ok_or_else(|| anyhow::anyhow!("package does not contain manifest.json"))?;

    #[derive(Deserialize)]
    struct PackageManifest {
        routes: Vec<serde_json::Value>,
    }
    let manifest: PackageManifest =
        serde_json::from_slice(&manifest_bytes).context("failed to parse manifest.json")?;

    // Upload each WASM module and build a lookup: module-name → asset URI.
    // Store under both the raw stem (guest_example) and the dash form (guest-example).
    let mut module_uris: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut imported_modules: Vec<ImportedModule> = Vec::new();

    for (stem, bytes) in &wasm_files {
        let uri = push_asset_bytes(stem, bytes)
            .await
            .with_context(|| format!("failed to upload module `{stem}`"))?;
        let dash_name = stem.replace('_', "-");
        module_uris.insert(stem.clone(), uri.clone());
        module_uris.insert(dash_name.clone(), uri.clone());
        imported_modules.push(ImportedModule {
            name: dash_name,
            asset_uri: uri,
        });
    }

    // Process manifest routes: replace each module reference with its asset URI.
    let mut routes_to_add: Vec<serde_json::Value> = Vec::new();
    let mut skipped_modules: Vec<String> = Vec::new();

    for mut route in manifest.routes {
        let has_targets = route
            .get("targets")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());

        if has_targets {
            let targets = route["targets"].as_array().cloned().unwrap_or_default();
            let mut new_targets: Vec<serde_json::Value> = Vec::new();
            let mut any_resolved = false;
            for mut target in targets {
                let module_name = target
                    .get("module")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned();
                if let Some(uri) = module_uris.get(&module_name) {
                    target["module"] = serde_json::Value::String(uri.clone());
                    any_resolved = true;
                }
                new_targets.push(target);
            }
            if !any_resolved {
                let route_path = route
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_owned();
                skipped_modules.push(route_path);
                continue;
            }
            route["targets"] = serde_json::Value::Array(new_targets);
        } else {
            let name = route
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned();
            if let Some(uri) = module_uris.get(&name) {
                route["targets"] = serde_json::json!([{ "module": uri, "weight": 100 }]);
            } else {
                skipped_modules.push(name);
                continue;
            }
        }
        routes_to_add.push(route);
    }

    let routes_added = routes_to_add.len();

    if !routes_to_add.is_empty() {
        let mut config = get_manifest_config().await?;
        let routes = config
            .get_mut("routes")
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| anyhow::anyhow!("manifest has no routes array"))?;

        for new_route in &routes_to_add {
            let path = new_route.get("path").and_then(|v| v.as_str()).unwrap_or("");
            routes.retain(|r| r.get("path").and_then(|v| v.as_str()) != Some(path));
        }
        routes.extend(routes_to_add);
        patch_and_apply_manifest(config).await?;
    }

    Ok(ImportPackageResult {
        imported_modules,
        skipped_modules,
        routes_added,
    })
}

pub async fn push_large_model(file_path: &str) -> Result<String> {
    push_large_model_with_progress(file_path, |_| {}).await
}

pub async fn push_large_model_with_progress<F>(file_path: &str, progress: F) -> Result<String>
where
    F: FnMut(f32) + Send,
{
    let mut progress = progress;
    push_large_model_with_status(file_path, move |status| {
        progress(status.percentage);
    })
    .await
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUploadPlanPreview {
    pub alias: String,
    pub files_included: usize,
    pub bytes_included: u64,
    pub files: Vec<String>,
}

pub async fn preview_large_model_upload(file_path: &str) -> Result<ModelUploadPlanPreview> {
    let source = std::path::Path::new(file_path);
    let metadata = tokio::fs::metadata(file_path)
        .await
        .with_context(|| format!("failed to read metadata for model `{file_path}`"))?;
    if metadata.is_file() && metadata.len() == 0 {
        anyhow::bail!("model payload `{file_path}` must not be empty");
    }

    let source_owned = source.to_path_buf();
    let archive_plan = tokio::task::spawn_blocking(move || model_archive_plan(&source_owned))
        .await
        .context("model archive scan task panicked")??;
    Ok(ModelUploadPlanPreview {
        alias: archive_plan.alias,
        files_included: archive_plan.files.len(),
        bytes_included: archive_plan.included_bytes,
        files: archive_plan
            .files
            .iter()
            .take(50)
            .map(|file| file.relative.to_string_lossy().into_owned())
            .collect(),
    })
}

pub async fn push_large_model_with_status<F>(file_path: &str, mut status: F) -> Result<String>
where
    F: FnMut(ModelUploadStatus) + Send,
{
    // The broker still receives a gzip+tar archive for compatibility, but
    // integrity is declared per extracted model file. This avoids a dry
    // compression pass over multi-GiB model directories before upload starts.
    status(ModelUploadStatus::stage("scanning", 0.0));
    let source = std::path::Path::new(file_path);
    let metadata = tokio::fs::metadata(file_path)
        .await
        .with_context(|| format!("failed to read metadata for model `{file_path}`"))?;
    if metadata.is_file() && metadata.len() == 0 {
        anyhow::bail!("model payload `{file_path}` must not be empty");
    }

    let source_owned = source.to_path_buf();
    let archive_plan = tokio::task::spawn_blocking(move || model_archive_plan(&source_owned))
        .await
        .context("model archive scan task panicked")??;
    for file in archive_plan.files.iter().take(25) {
        status(ModelUploadStatus::included(
            &archive_plan.alias,
            archive_plan.files.len(),
            archive_plan.included_bytes,
            file,
        ));
    }
    status(ModelUploadStatus::planned(&archive_plan));

    let upload_limit_bytes = max_model_archive_upload_bytes(&archive_plan);
    let (prep_tx, mut prep_rx) = tokio::sync::mpsc::channel::<ModelUploadStatus>(8);
    let metadata_task = tokio::task::spawn_blocking(move || {
        model_archive_file_manifest_with_progress(archive_plan, upload_limit_bytes, prep_tx)
    });
    while let Some(prep_status) = prep_rx.recv().await {
        status(prep_status);
    }
    let archive_metadata = metadata_task
        .await
        .context("model archive metadata task panicked")??;
    status(ModelUploadStatus::prepared(&archive_metadata));

    upload_model_archive_streaming(archive_metadata, status).await
}

#[derive(Clone, Debug)]
struct ModelArchivePlan {
    alias: String,
    included_bytes: u64,
    files: Vec<ModelArchiveFile>,
}

#[derive(Clone, Debug)]
struct ModelArchiveMetadata {
    plan: ModelArchivePlan,
    file_manifest: Vec<InitModelUploadFile>,
    size_bytes: u64,
}

impl ModelArchiveMetadata {
    fn alias(&self) -> &str {
        &self.plan.alias
    }

    fn included_bytes(&self) -> u64 {
        self.plan.included_bytes
    }

    fn files(&self) -> &[ModelArchiveFile] {
        &self.plan.files
    }
}

#[derive(Clone, Debug)]
struct ModelArchiveFile {
    path: std::path::PathBuf,
    relative: std::path::PathBuf,
    size_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUploadStatus {
    pub phase: String,
    pub percentage: f32,
    pub alias: Option<String>,
    pub file: Option<String>,
    pub files_included: usize,
    pub bytes_included: u64,
    pub archive_bytes: Option<u64>,
    pub processed_bytes: Option<u64>,
    pub uploaded_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub part: Option<u64>,
}

impl ModelUploadStatus {
    fn stage(phase: &str, percentage: f32) -> Self {
        Self {
            phase: phase.to_owned(),
            percentage,
            alias: None,
            file: None,
            files_included: 0,
            bytes_included: 0,
            archive_bytes: None,
            processed_bytes: None,
            uploaded_bytes: None,
            total_bytes: None,
            part: None,
        }
    }

    fn included(
        alias: &str,
        files_included: usize,
        bytes_included: u64,
        file: &ModelArchiveFile,
    ) -> Self {
        Self {
            phase: "included".to_owned(),
            percentage: 0.0,
            alias: Some(alias.to_owned()),
            file: Some(file.relative.to_string_lossy().into_owned()),
            files_included,
            bytes_included,
            archive_bytes: None,
            processed_bytes: None,
            uploaded_bytes: None,
            total_bytes: None,
            part: None,
        }
    }

    fn planned(plan: &ModelArchivePlan) -> Self {
        Self {
            phase: "preparing".to_owned(),
            percentage: 0.0,
            alias: Some(plan.alias.clone()),
            file: None,
            files_included: plan.files.len(),
            bytes_included: plan.included_bytes,
            archive_bytes: None,
            processed_bytes: None,
            uploaded_bytes: None,
            total_bytes: None,
            part: None,
        }
    }

    fn preparing_progress(
        alias: &str,
        files_included: usize,
        bytes_included: u64,
        processed_bytes: u64,
    ) -> Self {
        let percentage = if bytes_included == 0 {
            0.0
        } else {
            ((processed_bytes as f64 / bytes_included as f64) * 100.0).min(99.0) as f32
        };
        Self {
            phase: "preparing".to_owned(),
            percentage,
            alias: Some(alias.to_owned()),
            file: None,
            files_included,
            bytes_included,
            archive_bytes: None,
            processed_bytes: Some(processed_bytes),
            uploaded_bytes: None,
            total_bytes: Some(bytes_included),
            part: None,
        }
    }

    fn prepared(archive: &ModelArchiveMetadata) -> Self {
        Self {
            phase: "prepared".to_owned(),
            percentage: 0.0,
            alias: Some(archive.alias().to_owned()),
            file: None,
            files_included: archive.files().len(),
            bytes_included: archive.included_bytes(),
            archive_bytes: Some(archive.size_bytes),
            processed_bytes: Some(archive.size_bytes),
            uploaded_bytes: None,
            total_bytes: Some(archive.size_bytes),
            part: None,
        }
    }

    fn uploading(archive: &ModelArchiveMetadata, uploaded_bytes: u64, part: u64) -> Self {
        Self {
            phase: "uploading".to_owned(),
            percentage: ((uploaded_bytes as f64 / archive.size_bytes as f64) * 100.0) as f32,
            alias: Some(archive.alias().to_owned()),
            file: None,
            files_included: archive.files().len(),
            bytes_included: archive.included_bytes(),
            archive_bytes: Some(archive.size_bytes),
            processed_bytes: None,
            uploaded_bytes: Some(uploaded_bytes),
            total_bytes: Some(archive.size_bytes),
            part: Some(part),
        }
    }

    fn committing(archive: &ModelArchiveMetadata) -> Self {
        Self {
            phase: "committing".to_owned(),
            percentage: 100.0,
            alias: Some(archive.alias().to_owned()),
            file: None,
            files_included: archive.files().len(),
            bytes_included: archive.included_bytes(),
            archive_bytes: Some(archive.size_bytes),
            processed_bytes: None,
            uploaded_bytes: Some(archive.size_bytes),
            total_bytes: Some(archive.size_bytes),
            part: None,
        }
    }
}

fn model_archive_plan(source: &std::path::Path) -> Result<ModelArchivePlan> {
    let is_dir = source.is_dir();
    let alias = derive_model_alias(source, is_dir);
    let files = collect_model_archive_files(source)?;
    if files.is_empty() {
        anyhow::bail!(
            "model directory `{}` does not contain supported model files",
            source.display()
        );
    }
    let included_bytes = files.iter().map(|file| file.size_bytes).sum();
    Ok(ModelArchivePlan {
        alias,
        included_bytes,
        files,
    })
}

fn model_archive_file_manifest_with_progress(
    plan: ModelArchivePlan,
    upload_limit_bytes: u64,
    progress_tx: tokio::sync::mpsc::Sender<ModelUploadStatus>,
) -> Result<ModelArchiveMetadata> {
    let progress = ArchivePreparationProgress::new(&plan, progress_tx);
    let file_manifest = hash_model_files_with_progress(&plan.files, progress)?;
    Ok(ModelArchiveMetadata {
        plan,
        file_manifest,
        size_bytes: upload_limit_bytes,
    })
}

fn max_model_archive_upload_bytes(plan: &ModelArchivePlan) -> u64 {
    let one_percent = plan.included_bytes / 100;
    let per_file_overhead = (plan.files.len() as u64).saturating_mul(4096);
    plan.included_bytes
        .saturating_add(one_percent)
        .saturating_add(per_file_overhead)
        .saturating_add(16 * 1024 * 1024)
}

fn write_model_archive<W: Write>(files: &[ModelArchiveFile], writer: W) -> Result<usize> {
    let encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(writer, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    for file in files {
        builder
            .append_path_with_name(&file.path, &file.relative)
            .with_context(|| format!("failed to archive model file `{}`", file.path.display()))?;
    }
    builder
        .into_inner()
        .context("failed to finalize model upload tar stream")?
        .finish()
        .context("failed to finalize model upload gzip stream")?;
    Ok(files.len())
}

fn hash_model_files_with_progress(
    files: &[ModelArchiveFile],
    mut progress: ArchivePreparationProgress,
) -> Result<Vec<InitModelUploadFile>> {
    let mut manifest = Vec::with_capacity(files.len());
    let mut processed_bytes = 0_u64;
    for file in files {
        manifest.push(hash_model_file(
            file,
            Some((&mut progress, &mut processed_bytes)),
        )?);
    }
    Ok(manifest)
}

fn hash_model_file(
    file: &ModelArchiveFile,
    mut progress: Option<(&mut ArchivePreparationProgress, &mut u64)>,
) -> Result<InitModelUploadFile> {
    let mut input = std::fs::File::open(&file.path)
        .with_context(|| format!("failed to open model file `{}`", file.path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .with_context(|| format!("failed to read model file `{}`", file.path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        if let Some((progress, processed_bytes)) = progress.as_mut() {
            **processed_bytes = processed_bytes.saturating_add(read as u64);
            progress.record(**processed_bytes);
        }
    }
    Ok(InitModelUploadFile {
        path: archive_manifest_path(&file.relative),
        size_bytes: file.size_bytes,
        sha256: format!("sha256:{}", hex::encode(hasher.finalize())),
    })
}

fn archive_manifest_path(path: &std::path::Path) -> String {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

struct ArchivePreparationProgress {
    alias: String,
    files_included: usize,
    bytes_included: u64,
    next_emit_bytes: u64,
    tx: tokio::sync::mpsc::Sender<ModelUploadStatus>,
}

impl ArchivePreparationProgress {
    fn new(plan: &ModelArchivePlan, tx: tokio::sync::mpsc::Sender<ModelUploadStatus>) -> Self {
        Self {
            alias: plan.alias.clone(),
            files_included: plan.files.len(),
            bytes_included: plan.included_bytes,
            next_emit_bytes: MODEL_PREP_PROGRESS_BYTES,
            tx,
        }
    }

    fn record(&mut self, archive_bytes: u64) {
        if archive_bytes < self.next_emit_bytes {
            return;
        }
        while self.next_emit_bytes <= archive_bytes {
            self.next_emit_bytes = self
                .next_emit_bytes
                .saturating_add(MODEL_PREP_PROGRESS_BYTES);
        }
        let _ = self.tx.blocking_send(ModelUploadStatus::preparing_progress(
            &self.alias,
            self.files_included,
            self.bytes_included,
            archive_bytes,
        ));
    }
}

fn collect_model_archive_files(source: &std::path::Path) -> Result<Vec<ModelArchiveFile>> {
    if source.is_file() {
        let relative = source
            .file_name()
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("model.bin"));
        return Ok(vec![ModelArchiveFile {
            path: source.to_path_buf(),
            relative,
            size_bytes: std::fs::metadata(source)
                .with_context(|| format!("failed to read metadata for `{}`", source.display()))?
                .len(),
        }]);
    }

    let mut files = Vec::new();
    collect_model_archive_files_from_dir(source, std::path::Path::new(""), &mut files)?;
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

fn collect_model_archive_files_from_dir(
    source: &std::path::Path,
    relative: &std::path::Path,
    files: &mut Vec<ModelArchiveFile>,
) -> Result<()> {
    let mut entries = std::fs::read_dir(source)
        .with_context(|| format!("failed to read model directory `{}`", source.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to scan model directory `{}`", source.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_name = entry.file_name();
        if should_skip_model_archive_entry(&file_name) {
            continue;
        }

        let path = entry.path();
        let entry_relative = relative.join(&file_name);
        let metadata = entry
            .metadata()
            .with_context(|| format!("failed to read metadata for `{}`", path.display()))?;
        if metadata.is_dir() {
            collect_model_archive_files_from_dir(&path, &entry_relative, files)?;
        } else if metadata.is_file() && is_supported_model_archive_file(&path) {
            files.push(ModelArchiveFile {
                path,
                relative: entry_relative,
                size_bytes: metadata.len(),
            });
        }
    }
    Ok(())
}

fn should_skip_model_archive_entry(name: &std::ffi::OsStr) -> bool {
    matches!(name.to_str(), Some(".git" | ".hg" | ".svn"))
}

fn is_supported_model_archive_file(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let lower_name = name.to_ascii_lowercase();
    if matches!(
        lower_name.as_str(),
        "merges.txt"
            | "chat_template.jinja"
            | "tokenizer.model"
            | "spiece.model"
            | "sentencepiece.bpe.model"
            | ".quant_summary.txt"
    ) {
        return true;
    }

    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "safetensors" | "gguf" | "bin" | "pt" | "pth" | "onnx" | "json" | "model" | "tiktoken"
    )
}

/// Derive a safe model alias (a single path component) from the source name.
fn derive_model_alias(source: &std::path::Path, is_dir: bool) -> String {
    let raw = if is_dir {
        source.file_name()
    } else {
        source.file_stem()
    }
    .and_then(|name| name.to_str())
    .unwrap_or("model");
    let mut alias: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while alias.starts_with('.') {
        alias.remove(0);
    }
    if alias.is_empty() {
        "model".to_owned()
    } else {
        alias
    }
}

async fn upload_model_archive_streaming<F>(
    archive: ModelArchiveMetadata,
    mut status: F,
) -> Result<String>
where
    F: FnMut(ModelUploadStatus) + Send,
{
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let init_response = client
        .post(build_endpoint_url(&config.url, ADMIN_MODEL_INIT_PATH)?)
        .bearer_auth(&config.token)
        .json(&InitModelUploadRequest {
            size_bytes: archive.size_bytes,
            alias: Some(archive.alias()),
            files: archive.file_manifest.clone(),
        })
        .send()
        .await
        .with_context(|| {
            format!(
                "failed to initialize model upload for `{}`",
                archive.alias()
            )
        })?;
    let init_status = init_response.status();
    let init_body = init_response
        .bytes()
        .await
        .context("failed to read model-init response body")?;
    if !init_status.is_success() {
        anyhow::bail!(
            "model upload init failed with {init_status}: {}",
            String::from_utf8_lossy(&init_body).trim()
        );
    }
    let init_payload: InitModelUploadResponse = serde_json::from_slice(&init_body)
        .context("failed to decode model-init response payload")?;

    let (chunk_tx, mut chunk_rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, String>>(2);
    let files = archive.files().to_vec();
    let compression_task = tokio::task::spawn_blocking(move || {
        let mut writer = ChunkSenderWriter::new(chunk_tx);
        let result = write_model_archive(&files, &mut writer)
            .and_then(|_| writer.finish())
            .map(|_| ());
        if let Err(error) = result {
            let _ = writer.send_error(error.to_string());
        }
    });

    let mut uploaded_bytes = 0_u64;
    let mut part = 1_u64;
    while let Some(chunk) = chunk_rx.recv().await {
        let chunk = chunk.map_err(|error| anyhow::anyhow!(error))?;
        let chunk_len = chunk.len() as u64;
        let upload_url = format!(
            "{}/{}?part={part}",
            build_endpoint_url(&config.url, ADMIN_MODEL_UPLOAD_PATH)?,
            init_payload.upload_id
        );
        let response = client
            .put(upload_url)
            .bearer_auth(&config.token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(chunk)
            .send()
            .await
            .with_context(|| {
                format!(
                    "failed to upload chunk {part} for model `{}`",
                    archive.alias()
                )
            })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "failed to read upload error body".to_owned());
            anyhow::bail!("model chunk upload failed with {status}: {}", body.trim());
        }

        uploaded_bytes = uploaded_bytes.saturating_add(chunk_len);
        status(ModelUploadStatus::uploading(&archive, uploaded_bytes, part));
        part = part.saturating_add(1);
    }
    compression_task
        .await
        .context("model archive streaming task panicked")?;
    status(ModelUploadStatus::committing(&archive));

    let commit_url = format!(
        "{}/{}",
        build_endpoint_url(&config.url, ADMIN_MODEL_COMMIT_PATH)?,
        init_payload.upload_id
    );
    let commit_response = client
        .post(commit_url)
        .bearer_auth(&config.token)
        .send()
        .await
        .with_context(|| format!("failed to commit model upload for `{}`", archive.alias()))?;
    let commit_status = commit_response.status();
    let commit_body = commit_response
        .bytes()
        .await
        .context("failed to read model-commit response body")?;
    if !commit_status.is_success() {
        anyhow::bail!(
            "model upload commit failed with {commit_status}: {}",
            String::from_utf8_lossy(&commit_body).trim()
        );
    }

    status(ModelUploadStatus::stage("done", 100.0));
    let payload: CommitModelUploadResponse = serde_json::from_slice(&commit_body)
        .context("failed to decode model-commit response payload")?;
    Ok(payload.model_path)
}

struct ChunkSenderWriter {
    tx: tokio::sync::mpsc::Sender<Result<Vec<u8>, String>>,
    buffer: Vec<u8>,
}

impl ChunkSenderWriter {
    fn new(tx: tokio::sync::mpsc::Sender<Result<Vec<u8>, String>>) -> Self {
        Self {
            tx,
            buffer: Vec::with_capacity(MODEL_CHUNK_BYTES),
        }
    }

    fn finish(&mut self) -> Result<()> {
        if !self.buffer.is_empty() {
            let chunk = std::mem::take(&mut self.buffer);
            self.send_chunk(chunk)?;
        }
        Ok(())
    }

    fn send_error(&mut self, error: String) -> Result<()> {
        self.tx
            .blocking_send(Err(error))
            .map_err(|_| anyhow::anyhow!("model upload receiver closed"))
    }

    fn send_chunk(&mut self, chunk: Vec<u8>) -> std::io::Result<()> {
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| std::io::Error::new(ErrorKind::BrokenPipe, "model upload receiver closed"))
    }
}

impl Write for ChunkSenderWriter {
    fn write(&mut self, mut buf: &[u8]) -> std::io::Result<usize> {
        let original_len = buf.len();
        while !buf.is_empty() {
            let available = MODEL_CHUNK_BYTES - self.buffer.len();
            let take = available.min(buf.len());
            self.buffer.extend_from_slice(&buf[..take]);
            buf = &buf[take..];
            if self.buffer.len() == MODEL_CHUNK_BYTES {
                let chunk = std::mem::take(&mut self.buffer);
                self.send_chunk(chunk)?;
            }
        }
        Ok(original_len)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn current_connection() -> Option<InstanceConfig> {
    connection_state()
        .read()
        .expect("connection state should not be poisoned")
        .clone()
}

fn current_operator_name() -> Option<String> {
    operator_state()
        .read()
        .expect("operator state should not be poisoned")
        .clone()
}

fn require_connection() -> Result<InstanceConfig> {
    current_connection().ok_or_else(|| anyhow::anyhow!("no active Tachyon node connection"))
}

fn parse_engine_status(raw_lockfile: &str) -> Result<String> {
    let manifest: IntegrityManifest =
        serde_json::from_str(raw_lockfile).context("failed to parse integrity.lock manifest")?;
    let sealed_config: SealedConfig = serde_json::from_str(&manifest.config_payload)
        .context("failed to parse sealed config payload")?;

    Ok(format!(
        "routes={} batch_targets={} status=ready",
        sealed_config.routes.len(),
        sealed_config.batch_targets.len()
    ))
}

fn parse_route_summary(route: &serde_json::Value) -> Option<MeshRouteSummary> {
    let path = route.get("path")?.as_str()?.to_owned();
    let name = route
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| path.trim_start_matches('/'))
        .to_owned();
    let role = route
        .get("role")
        .and_then(|value| value.as_str())
        .unwrap_or("user")
        .to_owned();
    let target_count = route
        .get("targets")
        .and_then(|value| value.as_array())
        .map(|targets| targets.len().max(1))
        .unwrap_or(1);
    let requires_tee = route
        .get("requires_tee")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let encrypted_volume_count = route
        .get("volumes")
        .and_then(|value| value.as_array())
        .map(|volumes| {
            volumes
                .iter()
                .filter(|volume| {
                    volume
                        .get("encrypted")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);

    let allow_all_scopes = {
        let scopes = route.get("scopes");
        scopes.is_none()
            || scopes
                .and_then(|v| v.as_str())
                .map(|s| s == "allow-all")
                .unwrap_or(false)
    };

    Some(MeshRouteSummary {
        path,
        name,
        role,
        target_count,
        requires_tee,
        encrypted_volume_count,
        allow_all_scopes,
    })
}

fn batch_target_name(batch_target: &serde_json::Value) -> Option<String> {
    batch_target
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn read_local_system_manifest() -> Vec<RegisteredSystem> {
    let path = workspace_root().join("systems").join("manifest.toml");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_system_manifest(&raw)
}

fn parse_system_manifest(raw: &str) -> Vec<RegisteredSystem> {
    let mut systems = Vec::new();
    let mut current = RegisteredSystem::default();
    let mut in_entry = false;
    for line in raw.lines().map(str::trim) {
        if line == "[[system]]" {
            if in_entry {
                systems.push(current);
                current = RegisteredSystem::default();
            }
            in_entry = true;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "slug" => current.slug = value,
            "crate_name" => current.crate_name = value,
            "version" => current.version = value,
            "description" => current.description = value,
            _ => {}
        }
    }
    if in_entry {
        systems.push(current);
    }
    systems
}

fn deployed_system_from_nodes(system: &RegisteredSystem, nodes: &[EnrolledNode]) -> DeployedSystem {
    let deployed_nodes = nodes
        .iter()
        .flat_map(|node| {
            node.capabilities
                .active_systems
                .iter()
                .filter(move |active| active.slug == system.slug)
                .map(move |active| DeployedSystemNode {
                    node_id: node.node_id.clone(),
                    version: active.version.clone(),
                })
        })
        .collect::<Vec<_>>();
    let has_drift = deployed_nodes
        .iter()
        .any(|node| node.version != system.version);
    DeployedSystem {
        slug: system.slug.clone(),
        catalog_version: system.version.clone(),
        status: if deployed_nodes.is_empty() {
            "not-deployed".to_owned()
        } else if has_drift {
            "version-drift".to_owned()
        } else {
            "deployed".to_owned()
        },
        host_node_count: deployed_nodes.len(),
        has_drift,
        nodes: deployed_nodes,
    }
}

async fn fetch_remote_status(config: &InstanceConfig) -> Result<String> {
    let client = build_http_client(config)?;
    let response = client
        .get(build_endpoint_url(&config.url, ADMIN_STATUS_PATH)?)
        .bearer_auth(&config.token)
        .send()
        .await
        .with_context(|| format!("failed to reach Tachyon node at {}", config.url))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read admin status response body")?;

    if !status.is_success() {
        anyhow::bail!(
            "remote admin status request failed with {status}: {}",
            body.trim()
        );
    }

    Ok(body)
}

async fn get_admin_json<T>(path: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    get_admin_json_with_query(path, &[]).await
}

async fn get_admin_json_with_query<T>(path: &str, query: &[(&str, String)]) -> Result<T>
where
    T: DeserializeOwned,
{
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let mut url = reqwest::Url::parse(&build_endpoint_url(&config.url, path)?)
        .with_context(|| format!("invalid Tachyon admin endpoint `{path}`"))?;
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter());
    }
    let response = client
        .get(url)
        .bearer_auth(&config.token)
        .send()
        .await
        .with_context(|| format!("failed to query Tachyon admin endpoint `{path}`"))?;
    decode_admin_response(response, path).await
}

async fn post_admin_json<I, T>(path: &str, input: &I) -> Result<T>
where
    I: Serialize + ?Sized,
    T: DeserializeOwned,
{
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_endpoint_url(&config.url, path)?)
        .bearer_auth(&config.token)
        .json(input)
        .send()
        .await
        .with_context(|| format!("failed to post Tachyon admin endpoint `{path}`"))?;
    decode_admin_response(response, path).await
}

async fn patch_admin_json<I, T>(path: &str, input: &I) -> Result<T>
where
    I: Serialize + ?Sized,
    T: DeserializeOwned,
{
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .patch(build_endpoint_url(&config.url, path)?)
        .bearer_auth(&config.token)
        .json(input)
        .send()
        .await
        .with_context(|| format!("failed to patch Tachyon admin endpoint `{path}`"))?;
    decode_admin_response(response, path).await
}

async fn delete_admin_path(path: &str) -> Result<()> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .delete(build_endpoint_url(&config.url, path)?)
        .bearer_auth(&config.token)
        .send()
        .await
        .with_context(|| format!("failed to delete Tachyon admin endpoint `{path}`"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.bytes().await.unwrap_or_default();
        anyhow::bail!(
            "admin endpoint `{path}` failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }
    Ok(())
}

async fn decode_admin_response<T>(response: reqwest::Response, path: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    let status = response.status();
    let body = response
        .bytes()
        .await
        .with_context(|| format!("failed to read Tachyon admin endpoint `{path}` response"))?;
    if !status.is_success() {
        anyhow::bail!(
            "admin endpoint `{path}` failed with {status}: {}",
            String::from_utf8_lossy(&body).trim()
        );
    }

    serde_json::from_slice(&body)
        .with_context(|| format!("failed to decode Tachyon admin endpoint `{path}` response"))
}

fn build_http_client(config: &InstanceConfig) -> Result<reqwest::Client> {
    let cache_key = http_client_cache_key(config);
    if let Some(client) = http_client_cache()
        .read()
        .expect("HTTP client cache should not be poisoned")
        .get(&cache_key)
        .cloned()
    {
        return Ok(client);
    }

    let url = reqwest::Url::parse(&config.url)
        .with_context(|| format!("invalid Tachyon node URL `{}`", config.url))?;
    let mut builder = reqwest::Client::builder();

    if allows_insecure_local_tls(&url) {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(identity_bytes) = config.mtls_cert.as_deref() {
        builder = builder.identity(parse_identity(identity_bytes, config.mtls_key.as_deref())?);
    }

    let client = builder
        .build()
        .context("failed to build authenticated HTTP client")?;
    http_client_cache()
        .write()
        .expect("HTTP client cache should not be poisoned")
        .insert(cache_key, client.clone());
    Ok(client)
}

fn http_client_cache() -> &'static RwLock<BTreeMap<String, reqwest::Client>> {
    HTTP_CLIENT_CACHE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn http_client_cache_key(config: &InstanceConfig) -> String {
    let cert_hash = config
        .mtls_cert
        .as_deref()
        .map(short_hash)
        .unwrap_or_else(|| "none".to_owned());
    let key_hash = config
        .mtls_key
        .as_deref()
        .map(short_hash)
        .unwrap_or_else(|| "none".to_owned());
    format!("{}|cert:{cert_hash}|key:{key_hash}", config.url.trim())
}

fn short_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(&digest[..8])
}

fn parse_identity(cert_bytes: &[u8], key_bytes: Option<&[u8]>) -> Result<reqwest::Identity> {
    if let Some(key_bytes) = key_bytes {
        let mut identity_bundle = Vec::with_capacity(cert_bytes.len() + key_bytes.len() + 1);
        identity_bundle.extend_from_slice(cert_bytes);
        identity_bundle.push(b'\n');
        identity_bundle.extend_from_slice(key_bytes);
        reqwest::Identity::from_pem(&identity_bundle)
    } else {
        reqwest::Identity::from_pem(cert_bytes)
    }
    .context("failed to parse mTLS identity as a PEM certificate/private-key bundle")
}

fn build_endpoint_url(base_url: &str, path: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(base_url)
        .with_context(|| format!("invalid Tachyon node URL `{base_url}`"))?;
    url.set_path(path);
    url.set_query(None);
    Ok(url.to_string())
}

fn public_connection_config(url: &str, cert: Option<Vec<u8>>) -> InstanceConfig {
    InstanceConfig {
        url: url.trim().to_owned(),
        token: String::new(),
        mtls_cert: cert,
        mtls_key: None,
    }
}

fn allows_insecure_local_tls(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };

    if host.eq_ignore_ascii_case("localhost")
        || host.eq_ignore_ascii_case("home-lab-k3s.wsl")
        || host.ends_with(".wsl")
    {
        return true;
    }

    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

fn validate_connection_config(config: &InstanceConfig) -> Result<()> {
    if config.url.is_empty() {
        anyhow::bail!("node URL must not be empty");
    }
    if config.token.is_empty() {
        anyhow::bail!("admin token must not be empty");
    }
    reqwest::Url::parse(&config.url)
        .with_context(|| format!("invalid Tachyon node URL `{}`", config.url))?;
    Ok(())
}

fn normalize_operator_name(username: &str) -> Result<String> {
    let trimmed = username.trim();
    if trimmed.is_empty() {
        anyhow::bail!("operator username must not be empty");
    }

    Ok(trimmed.to_owned())
}

fn sha256_hash(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_upload_chunks_are_large_enough_for_local_uploads() {
        assert_eq!(MODEL_CHUNK_BYTES, 16 * 1024 * 1024);
    }

    #[test]
    fn http_client_cache_key_tracks_endpoint_and_identity() {
        let base = InstanceConfig {
            url: " https://127.0.0.1:9443 ".to_owned(),
            token: "token-a".to_owned(),
            mtls_cert: Some(b"cert-a".to_vec()),
            mtls_key: Some(b"key-a".to_vec()),
        };
        let same_transport = InstanceConfig {
            token: "token-b".to_owned(),
            ..base.clone()
        };
        let different_identity = InstanceConfig {
            mtls_cert: Some(b"cert-b".to_vec()),
            ..base.clone()
        };

        assert_eq!(
            http_client_cache_key(&base),
            http_client_cache_key(&same_transport)
        );
        assert_ne!(
            http_client_cache_key(&base),
            http_client_cache_key(&different_identity)
        );
    }

    #[test]
    fn available_model_from_openai_id_splits_engine_and_alias() {
        assert_eq!(
            available_model_from_openai_id("gguf/llama-3"),
            AvailableAiModel {
                id: "gguf/llama-3".to_owned(),
                alias: "llama-3".to_owned(),
                engine: "gguf".to_owned(),
            }
        );
        assert_eq!(
            available_model_from_openai_id("legacy-model"),
            AvailableAiModel {
                id: "legacy-model".to_owned(),
                alias: "legacy-model".to_owned(),
                engine: "unknown".to_owned(),
            }
        );
    }

    #[test]
    fn derive_model_alias_sanitizes_names() {
        assert_eq!(
            derive_model_alias(std::path::Path::new("/models/TinyLlama-1.1B"), true),
            "TinyLlama-1.1B"
        );
        // Single file → stem without extension.
        assert_eq!(
            derive_model_alias(std::path::Path::new("/models/llama3.gguf"), false),
            "llama3"
        );
        // Disallowed characters become `-`, leading dots stripped.
        assert_eq!(
            derive_model_alias(std::path::Path::new("/models/.weird name!"), true),
            "weird-name-"
        );
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn build_model_archive_round_trips_a_directory() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachyon-client-archive-{stamp}"));
        std::fs::create_dir_all(&dir).expect("temp model dir");
        std::fs::write(dir.join("config.json"), br#"{"model_type":"llama"}"#).unwrap();
        std::fs::write(dir.join("model.safetensors"), b"\x00\x01\x02").unwrap();

        let plan = model_archive_plan(&dir).expect("plan should build");
        assert_eq!(plan.alias, dir.file_name().unwrap().to_str().unwrap());
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let manifest =
            hash_model_files_with_progress(&plan.files, ArchivePreparationProgress::new(&plan, tx))
                .expect("file manifest should build");

        // The archive is a gzip stream that untars back to the same files.
        let archive = model_archive_bytes(&dir);
        let second_archive = model_archive_bytes(&dir);
        assert_eq!(sha256_hash(&archive), sha256_hash(&second_archive));
        assert_eq!(manifest.len(), 2);
        assert!(manifest
            .iter()
            .any(|file| file.path == "model.safetensors"
                && file.sha256 == sha256_hash(b"\x00\x01\x02")));
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(&archive[..]));
        let mut names: Vec<String> = tar
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .trim_start_matches("./")
                    .to_owned()
            })
            .filter(|name| !name.is_empty())
            .collect();
        names.sort();
        assert!(names.contains(&"config.json".to_owned()));
        assert!(names.contains(&"model.safetensors".to_owned()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn model_archive_plan_detects_sharded_safetensors_immediately() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachyon-client-qwen-plan-{stamp}"));
        std::fs::create_dir_all(dir.join(".git").join("lfs")).expect("temp git dir");
        std::fs::write(dir.join("config.json"), br#"{"model_type":"qwen3"}"#).unwrap();
        std::fs::write(dir.join("hf_quant_config.json"), br#"{"quantization":{}}"#).unwrap();
        std::fs::write(
            dir.join("model.safetensors.index.json"),
            br#"{"weight_map":{}}"#,
        )
        .unwrap();
        std::fs::write(dir.join("model-00001-of-00003.safetensors"), b"one").unwrap();
        std::fs::write(dir.join("model-00002-of-00003.safetensors"), b"two").unwrap();
        std::fs::write(dir.join("README.md"), b"ignored").unwrap();

        let plan = model_archive_plan(&dir).expect("plan should detect model files");
        let names: Vec<String> = plan
            .files
            .iter()
            .map(|file| file.relative.to_string_lossy().to_string())
            .collect();

        assert!(names.contains(&"config.json".to_owned()));
        assert!(names.contains(&"hf_quant_config.json".to_owned()));
        assert!(names.contains(&"model.safetensors.index.json".to_owned()));
        assert!(names.contains(&"model-00001-of-00003.safetensors".to_owned()));
        assert!(names.contains(&"model-00002-of-00003.safetensors".to_owned()));
        assert!(names.iter().all(|name| !name.contains(".git")));
        assert!(names.iter().all(|name| !name.ends_with("README.md")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn preview_large_model_upload_reports_included_files() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachyon-client-qwen-preview-{stamp}"));
        std::fs::create_dir_all(dir.join(".git").join("lfs")).expect("temp git dir");
        std::fs::write(dir.join("config.json"), br#"{"model_type":"qwen3"}"#).unwrap();
        std::fs::write(dir.join("model-00001-of-00003.safetensors"), b"one").unwrap();
        std::fs::write(dir.join("README.md"), b"ignored").unwrap();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let preview = runtime
            .block_on(preview_large_model_upload(dir.to_str().unwrap()))
            .expect("preview should detect model files");

        assert_eq!(preview.alias, dir.file_name().unwrap().to_str().unwrap());
        assert_eq!(preview.files_included, 2);
        assert_eq!(
            preview.files,
            vec![
                "config.json".to_owned(),
                "model-00001-of-00003.safetensors".to_owned()
            ]
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn build_model_archive_uses_model_file_allowlist() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tachyon-client-allowlist-archive-{stamp}"));
        let git_lfs_dir = dir.join(".git").join("lfs").join("objects");
        std::fs::create_dir_all(&git_lfs_dir).expect("temp git lfs dir");
        std::fs::write(dir.join("config.json"), br#"{"model_type":"llama"}"#).unwrap();
        std::fs::write(dir.join("model.safetensors"), b"\x00\x01\x02").unwrap();
        std::fs::write(dir.join("merges.txt"), b"merge rules").unwrap();
        std::fs::write(dir.join("README.md"), b"not needed at runtime").unwrap();
        std::fs::write(dir.join(".gitattributes"), b"*.safetensors filter=lfs").unwrap();
        std::fs::write(dir.join("training.log"), b"not a model artifact").unwrap();
        std::fs::write(git_lfs_dir.join("duplicate-object"), b"large duplicate").unwrap();

        let archive = model_archive_bytes(&dir);

        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(&archive[..]));
        let names: Vec<String> = tar
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|name| !name.contains(".git")));
        assert!(names.iter().any(|name| name.ends_with("config.json")));
        assert!(names.iter().any(|name| name.ends_with("model.safetensors")));
        assert!(names.iter().any(|name| name.ends_with("merges.txt")));
        assert!(names.iter().all(|name| !name.ends_with("README.md")));
        assert!(names.iter().all(|name| !name.ends_with(".gitattributes")));
        assert!(names.iter().all(|name| !name.ends_with("training.log")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn model_archive_bytes(source: &std::path::Path) -> Vec<u8> {
        let mut archive = Vec::new();
        let files = collect_model_archive_files(source).expect("files should collect");
        write_model_archive(&files, &mut archive).expect("archive should build");
        archive
    }

    #[test]
    fn parse_engine_status_summarizes_routes_and_batches() {
        let raw_lockfile = r#"{
          "config_payload": "{\"routes\":[{\"path\":\"/api/a\"},{\"path\":\"/api/b\"}],\"batch_targets\":[{\"name\":\"gc-job\"}]}",
          "public_key": "0000000000000000000000000000000000000000000000000000000000000000",
          "signature": "fixture-signature"
        }"#;

        let status = parse_engine_status(raw_lockfile).expect("status should parse");

        assert_eq!(status, "routes=2 batch_targets=1 status=ready");
    }

    #[test]
    fn admin_url_reuses_origin_and_overrides_path() {
        let url = build_endpoint_url("https://127.0.0.1:4000/ui?stale=1", "/admin/status")
            .expect("admin URL should build");

        assert_eq!(url, "https://127.0.0.1:4000/admin/status");
    }

    #[test]
    fn allows_insecure_tls_for_loopback_and_homelab_wsl_hosts() {
        let loopback = reqwest::Url::parse("https://127.0.0.1:4000").expect("loopback URL");
        let homelab = reqwest::Url::parse("https://home-lab-k3s.wsl").expect("homelab URL");
        let nested = reqwest::Url::parse("https://edge.home-lab-k3s.wsl").expect("nested URL");
        let wsl_private_ip =
            reqwest::Url::parse("https://172.18.194.89:20001").expect("WSL private IP URL");

        assert!(allows_insecure_local_tls(&loopback));
        assert!(allows_insecure_local_tls(&homelab));
        assert!(allows_insecure_local_tls(&nested));
        assert!(allows_insecure_local_tls(&wsl_private_ip));
    }

    #[test]
    fn denies_insecure_tls_for_non_local_hosts() {
        let public = reqwest::Url::parse("https://example.com").expect("public URL");

        assert!(!allows_insecure_local_tls(&public));
    }

    #[test]
    fn route_summary_defaults_to_single_target() {
        let route = json!({
            "path": "/api/demo",
            "role": "system",
            "name": "demo"
        });

        let summary = parse_route_summary(&route).expect("route summary should parse");

        assert_eq!(summary.path, "/api/demo");
        assert_eq!(summary.name, "demo");
        assert_eq!(summary.role, "system");
        assert_eq!(summary.target_count, 1);
        assert!(!summary.requires_tee);
        assert_eq!(summary.encrypted_volume_count, 0);
    }

    #[test]
    fn route_summary_marks_confidential_routes() {
        let route = json!({
            "path": "/api/secure",
            "requires_tee": true,
            "targets": [{ "module": "secure" }],
            "volumes": [
                { "host_path": "/tmp/plain", "guest_path": "/plain" },
                { "host_path": "/tmp/secure", "guest_path": "/secure", "encrypted": true }
            ]
        });

        let summary = parse_route_summary(&route).expect("route summary should parse");

        assert!(summary.requires_tee);
        assert_eq!(summary.encrypted_volume_count, 1);
    }

    #[test]
    fn batch_target_name_extracts_name() {
        let batch_target = json!({ "name": "gc-job" });

        assert_eq!(batch_target_name(&batch_target), Some("gc-job".to_owned()));
    }

    #[test]
    fn parse_sealed_external_resource() {
        let value = json!({
            "type": "external",
            "target": "https://api.example.com",
            "allowed_methods": ["GET", "POST"],
        });
        let resource = parse_sealed_resource("example", &value).expect("resource should parse");
        assert_eq!(resource.name, "example");
        assert_eq!(resource.kind, "external");
        assert_eq!(resource.target, "https://api.example.com");
        assert_eq!(resource.allowed_methods, vec!["GET", "POST"]);
        assert!(!resource.pending);
        assert!(resource.version_constraint.is_none());
    }

    #[test]
    fn parse_sealed_internal_resource() {
        let value = json!({
            "type": "internal",
            "target": "wasm://local",
            "version_constraint": "^1.0",
        });
        let resource = parse_sealed_resource("local", &value).expect("resource should parse");
        assert_eq!(resource.kind, "internal");
        assert_eq!(resource.version_constraint.as_deref(), Some("^1.0"));
        assert!(resource.allowed_methods.is_empty());
    }

    #[test]
    fn validate_external_target_rejects_plain_http() {
        let result = validate_external_target("github", "http://api.github.com");
        assert!(result.is_err(), "plain http public hosts must be rejected");
    }

    #[test]
    fn validate_external_target_accepts_https() {
        validate_external_target("github", "https://api.github.com").expect("https accepted");
    }

    #[test]
    fn validate_external_target_accepts_localhost_http() {
        validate_external_target("local", "http://localhost:9000").expect("loopback accepted");
    }

    #[test]
    fn validate_external_target_accepts_cluster_local_http() {
        validate_external_target("svc", "http://my-svc.namespace.svc.cluster.local")
            .expect("cluster-local accepted");
    }

    #[test]
    fn validate_resource_input_rejects_empty_name() {
        let input = MeshResourceInput {
            name: "  ".to_owned(),
            kind: "external".to_owned(),
            target: "https://api.example.com".to_owned(),
            allowed_methods: None,
            version_constraint: None,
        };
        assert!(validate_resource_input(&input).is_err());
    }

    #[test]
    fn validate_resource_input_normalizes_methods() {
        let input = MeshResourceInput {
            name: "stripe".to_owned(),
            kind: "external".to_owned(),
            target: "https://api.stripe.com".to_owned(),
            allowed_methods: Some(vec!["get".to_owned(), "POST".to_owned(), "get".to_owned()]),
            version_constraint: None,
        };
        let resource = validate_resource_input(&input).expect("input is valid");
        assert_eq!(resource.allowed_methods, vec!["GET", "POST"]);
        assert!(resource.pending);
    }
}
