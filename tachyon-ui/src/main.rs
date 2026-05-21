#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tauri_plugin_stronghold::stronghold::Stronghold;

const STRONGHOLD_PROFILE_CLIENT: &[u8] = b"tachyon-ui-auth";
const AUTH_PROFILE_RECORD: &[u8] = b"auth_profile";
const STRONGHOLD_PROFILE_KEY_BYTES: usize = 32;
const MFA_SESSION_TTL_SECONDS: u64 = 20 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthnLoginPayload {
    url: String,
    username: String,
    password: String,
    cert: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignupValidatePayload {
    url: String,
    token: String,
    cert: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignupStagePayload {
    url: String,
    token: String,
    first_name: String,
    last_name: String,
    username: String,
    password: String,
    cert: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignupFinalizePayload {
    url: String,
    session_id: String,
    totp_code: String,
    cert: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginFinalizePayload {
    url: String,
    session_id: String,
    totp_code: String,
    cert: Option<Vec<u8>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiResponse {
    success: bool,
    message: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedCredentials {
    url: String,
    username: String,
    password: String,
    #[serde(default)]
    pat: Option<String>,
    #[serde(default)]
    custom_ca: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SecureAuthProfile {
    url: String,
    username: String,
    password: String,
    pat: Option<String>,
    custom_ca: Option<Vec<u8>>,
}

struct StrongholdAuthStore {
    inner: Mutex<Stronghold>,
}

#[derive(Default)]
struct MfaSessionState {
    inner: Mutex<Option<MfaSessionToken>>,
}

#[derive(Clone)]
struct MfaSessionToken {
    token: String,
    expires_at: u64,
}

impl StrongholdAuthStore {
    fn new(snapshot_path: PathBuf, key: Vec<u8>) -> Result<Self, String> {
        let stronghold = Stronghold::new(snapshot_path, key)
            .map_err(|error| format!("failed to initialize Stronghold auth store: {error}"))?;
        Ok(Self {
            inner: Mutex::new(stronghold),
        })
    }

    fn get_record(&self, record: &[u8]) -> Result<Option<Vec<u8>>, String> {
        let stronghold = self
            .inner
            .lock()
            .map_err(|_| "Stronghold auth store lock is poisoned".to_owned())?;
        let client = load_or_create_stronghold_client(&stronghold)?;
        client
            .store()
            .get(record)
            .map_err(|error| format!("failed to read Stronghold auth record: {error}"))
    }

    fn insert_record(&self, record: &[u8], bytes: Vec<u8>) -> Result<(), String> {
        let stronghold = self
            .inner
            .lock()
            .map_err(|_| "Stronghold auth store lock is poisoned".to_owned())?;
        let client = load_or_create_stronghold_client(&stronghold)?;
        client
            .store()
            .insert(record.to_vec(), bytes, None)
            .map_err(|error| format!("failed to write Stronghold auth record: {error}"))?;
        stronghold
            .save()
            .map_err(|error| format!("failed to persist Stronghold auth snapshot: {error}"))
    }

    fn remove_record(&self, record: &[u8]) -> Result<(), String> {
        let stronghold = self
            .inner
            .lock()
            .map_err(|_| "Stronghold auth store lock is poisoned".to_owned())?;
        let client = load_or_create_stronghold_client(&stronghold)?;
        client
            .store()
            .delete(record)
            .map_err(|error| format!("failed to delete Stronghold auth record: {error}"))?;
        stronghold
            .save()
            .map_err(|error| format!("failed to persist Stronghold auth snapshot: {error}"))
    }
}

impl MfaSessionState {
    fn set(&self, token: tachyon_client::MfaSessionToken) -> Result<(), String> {
        let fallback_expires_at = current_unix_seconds()?.saturating_add(MFA_SESSION_TTL_SECONDS);
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "MFA session lock is poisoned".to_owned())?;
        *guard = Some(MfaSessionToken {
            token: token.mfa_session_token,
            expires_at: token.expires_at.max(fallback_expires_at),
        });
        Ok(())
    }

    fn require_valid(&self) -> Result<(), String> {
        let now = current_unix_seconds()?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| "MFA session lock is poisoned".to_owned())?;
        match guard.as_ref() {
            Some(session) if !session.token.trim().is_empty() && session.expires_at > now => Ok(()),
            Some(_) => {
                *guard = None;
                Err(
                    "MFA session expired; verify TOTP again before sealing configuration"
                        .to_owned(),
                )
            }
            None => Err("MFA step-up is required before sealing configuration".to_owned()),
        }
    }
}

fn current_unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())
        .map(|duration| duration.as_secs())
}

fn load_or_create_stronghold_client(
    stronghold: &Stronghold,
) -> Result<iota_stronghold::Client, String> {
    stronghold
        .get_client(STRONGHOLD_PROFILE_CLIENT)
        .or_else(|_| stronghold.load_client(STRONGHOLD_PROFILE_CLIENT))
        .or_else(|_| stronghold.create_client(STRONGHOLD_PROFILE_CLIENT))
        .map_err(|error| format!("failed to open Stronghold auth client: {error}"))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplyConfigurationResponse {
    success: bool,
    message: String,
    staged: bool,
    requires_seal: bool,
}

#[allow(dead_code)]
mod wit_contracts {
    pub mod routing {
        wit_bindgen::generate!({
            path: "../wit/config-routing.wit",
            world: "traffic-management-config",
        });
    }

    pub mod resilience {
        wit_bindgen::generate!({
            path: "../wit/config-resilience.wit",
            world: "resilience-chaos-config",
        });
    }

    pub mod ai {
        wit_bindgen::generate!({
            path: "../wit/config-ai.wit",
            world: "ai-orchestration-config",
        });
    }

    pub mod observability {
        wit_bindgen::generate!({
            path: "../wit/config-observability.wit",
            world: "observability-compute-config",
        });
    }

    pub mod storage {
        wit_bindgen::generate!({
            path: "../wit/config-storage.wit",
            world: "storage-state-config",
        });
    }

    pub mod fleet {
        wit_bindgen::generate!({
            path: "../wit/config-fleet.wit",
            world: "fleet-profile-config",
        });
    }
}

#[tauri::command]
async fn get_engine_status() -> Result<String, String> {
    tachyon_client::get_engine_status()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_mesh_graph() -> Result<tachyon_client::MeshGraphSnapshot, String> {
    tachyon_client::get_mesh_graph()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_volume_backups(
    route_path: String,
    guest_path: String,
) -> Result<Vec<tachyon_client::BackupSnapshot>, String> {
    tachyon_client::list_volume_backups(&route_path, &guest_path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn backup_volume(
    route_path: String,
    guest_path: String,
) -> Result<tachyon_client::BackupSnapshot, String> {
    tachyon_client::backup_volume(&route_path, &guest_path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn restore_volume(
    route_path: String,
    guest_path: String,
    snapshot_id: String,
) -> Result<(), String> {
    tachyon_client::restore_volume(&route_path, &guest_path, &snapshot_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_s3_volumes(route_path: String) -> Result<Vec<tachyon_client::S3VolumeEntry>, String> {
    tachyon_client::list_s3_volumes(&route_path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn attach_s3_volume(
    route_path: String,
    s3_url: String,
    guest_path: String,
    readonly: bool,
) -> Result<tachyon_client::S3VolumeEntry, String> {
    tachyon_client::attach_s3_volume(&route_path, &s3_url, &guest_path, readonly)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn detach_s3_volume(route_path: String, guest_path: String) -> Result<(), String> {
    tachyon_client::detach_s3_volume(&route_path, &guest_path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_topology_graph() -> Result<tachyon_client::TopologyGraphSpec, String> {
    tachyon_client::get_topology_graph()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_metrics() -> Result<tachyon_client::RuntimeMetrics, String> {
    tachyon_client::get_metrics()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn tail_logs(lines: Option<usize>) -> Result<Vec<tachyon_client::LogLine>, String> {
    tachyon_client::tail_logs(lines.unwrap_or(50))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_shadow_diffs() -> Result<Vec<tachyon_client::ShadowDiff>, String> {
    tachyon_client::get_shadow_diffs()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn connect_to_node(
    url: String,
    token: String,
    cert: Option<Vec<u8>>,
) -> Result<String, String> {
    tachyon_client::set_connection(url, token, cert).await?;
    tachyon_client::get_engine_status()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn authn_login(
    payload: AuthnLoginPayload,
) -> Result<tachyon_client::AuthLoginResponse, String> {
    tachyon_client::authn_login(
        &payload.url,
        &payload.username,
        &payload.password,
        payload.cert,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn finalize_login(
    payload: LoginFinalizePayload,
) -> Result<tachyon_client::AuthLoginResponse, String> {
    tachyon_client::finalize_login(
        &payload.url,
        &payload.session_id,
        &payload.totp_code,
        payload.cert,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn validate_signup_token(
    payload: SignupValidatePayload,
) -> Result<tachyon_client::RegistrationTokenClaims, String> {
    tachyon_client::validate_registration_token(&payload.url, &payload.token, payload.cert)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn stage_signup(
    payload: SignupStagePayload,
) -> Result<tachyon_client::StagedSignupSession, String> {
    tachyon_client::stage_signup(
        &payload.url,
        &payload.token,
        &payload.first_name,
        &payload.last_name,
        &payload.username,
        &payload.password,
        payload.cert,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn finalize_signup(
    payload: SignupFinalizePayload,
) -> Result<tachyon_client::AuthLoginResponse, String> {
    tachyon_client::finalize_enrollment(
        &payload.url,
        &payload.session_id,
        &payload.totp_code,
        payload.cert,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_list_users() -> Result<Vec<tachyon_client::IamUserSummary>, String> {
    tachyon_client::iam_list_users()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_update_user(
    username: String,
    update: tachyon_client::IamUserUpdate,
) -> Result<tachyon_client::IamUserSummary, String> {
    tachyon_client::iam_update_user(&username, &update)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_delete_user(username: String) -> Result<(), String> {
    tachyon_client::iam_delete_user(&username)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_list_groups() -> Result<Vec<tachyon_client::IamGroupSummary>, String> {
    tachyon_client::iam_list_groups()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_upsert_group(
    input: tachyon_client::IamGroupInput,
) -> Result<tachyon_client::IamGroupSummary, String> {
    tachyon_client::iam_upsert_group(&input)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_delete_group(name: String) -> Result<(), String> {
    tachyon_client::iam_delete_group(&name)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn fetch_user_audit_log(
    user: Option<String>,
    lines: Option<usize>,
) -> Result<Vec<tachyon_client::IamAuditEntry>, String> {
    tachyon_client::fetch_user_audit_log(user.as_deref(), lines.unwrap_or(50))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn fetch_canary_status() -> Result<Vec<tachyon_client::CanaryStatusEntry>, String> {
    tachyon_client::fetch_canary_status()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn abort_canary_rollout(route_path: String) -> Result<(), String> {
    tachyon_client::abort_canary_rollout(&route_path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn iam_regen_mfa(username: String) -> Result<Vec<String>, String> {
    tachyon_client::iam_regen_mfa(&username)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn generate_recovery_codes(username: String) -> Result<Vec<String>, String> {
    tachyon_client::generate_recovery_codes(&username)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn regenerate_account_security() -> Result<Vec<String>, String> {
    tachyon_client::regenerate_account_security()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn generate_pat(name: String, scopes: Vec<String>, ttl_days: u32) -> Result<String, String> {
    tachyon_client::generate_pat(&name, &scopes, ttl_days)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn generate_operator_invite() -> Result<tachyon_client::EnrollmentInvite, String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let node_public_key = format!("tachyon-ui-operator-invite-{timestamp}");
    tachyon_client::start_enrollment_invite(&node_public_key)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn push_asset(path: String, bytes: Option<Vec<u8>>) -> Result<String, String> {
    let result = if let Some(bytes) = bytes {
        tachyon_client::push_asset_bytes(&path, &bytes).await
    } else {
        tachyon_client::push_asset(&path).await
    };

    result.map_err(|error| error.to_string())
}

#[tauri::command]
async fn push_large_model(app: tauri::AppHandle, path: String) -> Result<String, String> {
    tachyon_client::push_large_model_with_progress(&path, |percentage| {
        let _ = app.emit("upload_progress", percentage);
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_resources() -> Result<Vec<tachyon_client::MeshResource>, String> {
    tachyon_client::read_resources()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_hardware_status() -> Result<tachyon_client::HardwareStatus, String> {
    Ok(tachyon_client::read_local_hardware_status())
}

#[tauri::command]
async fn list_enrolled_nodes() -> Result<Vec<tachyon_client::EnrolledNode>, String> {
    tachyon_client::list_enrolled_nodes()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_node_capabilities(
    node_id: String,
) -> Result<tachyon_client::NodeCapabilities, String> {
    tachyon_client::get_node_capabilities(&node_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_registered_systems() -> Result<Vec<tachyon_client::RegisteredSystem>, String> {
    tachyon_client::list_registered_systems()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn list_deployed_systems() -> Result<Vec<tachyon_client::DeployedSystem>, String> {
    tachyon_client::list_deployed_systems()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_cluster_hardware_summary() -> Result<tachyon_client::ClusterHardwareSummary, String> {
    tachyon_client::get_cluster_hardware_summary()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_staged_config(domain: String) -> Result<Option<serde_json::Value>, String> {
    tachyon_client::get_staged_config(&domain)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_active_config(domain: String) -> Result<Option<serde_json::Value>, String> {
    tachyon_client::get_active_config(&domain)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn toggle_system_route(slug: String, version: String, enabled: bool) -> Result<(), String> {
    tachyon_client::toggle_system_route(&slug, &version, enabled)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_staged_system_routes() -> Result<Vec<(String, bool)>, String> {
    tachyon_client::get_staged_system_routes()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn validate_hardware_policy(
    policy: tachyon_client::HardwarePolicy,
) -> Result<tachyon_client::HardwareValidation, String> {
    Ok(tachyon_client::validate_hardware_policy(&policy))
}

#[tauri::command]
async fn save_resource(
    resource: tachyon_client::MeshResourceInput,
) -> Result<tachyon_client::MeshResource, String> {
    tachyon_client::upsert_overlay_resource(resource)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn delete_resource(name: String) -> Result<(), String> {
    tachyon_client::remove_overlay_resource(&name)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn apply_configuration(
    domain: String,
    payload: Value,
    dry_run: Option<bool>,
) -> Result<ApplyConfigurationResponse, String> {
    let payload_for_staging = payload.clone();
    let response = match domain.as_str() {
        "config-routing" | "routing" => Ok(validate_traffic_config(&payload)),
        "config-resilience" | "resilience" => Ok(validate_resilience_config(&payload)),
        "config-ai" | "ai_orchestration" => Ok(validate_ai_config(&payload)),
        "config-security" | "identity" | "security-identity" => {
            Ok(validate_security_identity_config(&payload))
        }
        "config-rbac" | "rbac" => Ok(validate_rbac_panel_config(&payload)),
        "config-workloads" | "workloads" => Ok(validate_workloads_panel_config(&payload)),
        "config-observability" | "observability" => {
            Ok(validate_observability_panel_config(&payload))
        }
        "config-storage" | "storage" => Ok(validate_storage_panel_config(&payload)),
        "config-fleet" | "fleet" => Ok(validate_fleet_panel_config(&payload)),
        "config-assets" | "supply_chain" | "supply-chain" => {
            Ok(validate_supply_chain_panel_config(&payload))
        }
        _ => Err(format!("Unknown configuration domain: {domain}")),
    }?;

    if !response.success {
        return Ok(ApplyConfigurationResponse {
            success: false,
            message: response.message,
            staged: false,
            requires_seal: false,
        });
    }

    if dry_run.unwrap_or(false) {
        return Ok(ApplyConfigurationResponse {
            success: true,
            message: format!(
                "{} Dry-run only; no overlay state changed.",
                response.message
            ),
            staged: false,
            requires_seal: false,
        });
    }

    tachyon_client::stage_configuration_overlay(&domain, payload_for_staging)
        .await
        .map_err(|error| error.to_string())?;

    Ok(ApplyConfigurationResponse {
        success: true,
        message: format!(
            "{} Staged in local overlay; seal is required.",
            response.message
        ),
        staged: true,
        requires_seal: true,
    })
}

#[tauri::command]
async fn seal_and_apply_manifest(
    app: tauri::AppHandle,
) -> Result<tachyon_client::SealApplyOutcome, String> {
    let state = app
        .try_state::<MfaSessionState>()
        .ok_or_else(|| "MFA session backend is unavailable".to_owned())?;
    state.require_valid()?;
    tachyon_client::seal_and_apply_manifest()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_node_public_key() -> Result<String, String> {
    tachyon_client::get_node_public_key()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn bundle_and_apply_manifest(
    dependencies: Option<Vec<tachyon_client::BundleDependency>>,
) -> Result<tachyon_client::BundleApplyOutcome, String> {
    tachyon_client::bundle_and_apply_manifest(dependencies.unwrap_or_default())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_credentials(app: tauri::AppHandle, payload: SavedCredentials) -> Result<(), String> {
    let mut profile = read_secure_profile(&app).await?.unwrap_or_default();
    profile.url = payload.url;
    profile.username = payload.username;
    profile.password = payload.password;
    if payload.pat.is_some() {
        profile.pat = payload.pat;
    }
    if payload.custom_ca.is_some() {
        profile.custom_ca = payload.custom_ca;
    }
    write_secure_profile(&app, &profile).await
}

#[tauri::command]
async fn load_credentials(app: tauri::AppHandle) -> Result<Option<SavedCredentials>, String> {
    Ok(read_secure_profile(&app)
        .await?
        .map(|profile| SavedCredentials {
            url: profile.url,
            username: profile.username,
            password: profile.password,
            pat: profile.pat,
            custom_ca: profile.custom_ca,
        }))
}

#[tauri::command]
async fn delete_credentials(app: tauri::AppHandle) -> Result<(), String> {
    let store = app
        .try_state::<StrongholdAuthStore>()
        .ok_or_else(|| "Stronghold auth backend is unavailable".to_owned())?;
    store.remove_record(AUTH_PROFILE_RECORD)?;
    let path = legacy_secure_profile_path(&app)?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to delete legacy auth profile `{}`: {error}",
            path.display()
        )),
    }
}

#[tauri::command]
async fn save_custom_ca(app: tauri::AppHandle, cert: Vec<u8>) -> Result<(), String> {
    let mut profile = read_secure_profile(&app).await?.unwrap_or_default();
    profile.custom_ca = Some(cert);
    write_secure_profile(&app, &profile).await
}

#[tauri::command]
async fn load_custom_ca(app: tauri::AppHandle) -> Result<Option<Vec<u8>>, String> {
    Ok(read_secure_profile(&app)
        .await?
        .and_then(|profile| profile.custom_ca))
}

#[tauri::command]
async fn clear_custom_ca(app: tauri::AppHandle) -> Result<(), String> {
    let mut profile = read_secure_profile(&app).await?.unwrap_or_default();
    profile.custom_ca = None;
    write_secure_profile(&app, &profile).await
}

#[tauri::command]
async fn verify_session_totp(app: tauri::AppHandle, code: String) -> Result<(), String> {
    let trimmed = code.trim();
    if trimmed.len() != 6 || !trimmed.chars().all(|digit| digit.is_ascii_digit()) {
        return Err("MFA code must contain exactly 6 digits".to_owned());
    }

    let token = tachyon_client::verify_session_totp(trimmed)
        .await
        .map_err(|error| format!("step-up TOTP verification failed: {error}"))?;
    let state = app
        .try_state::<MfaSessionState>()
        .ok_or_else(|| "MFA session backend is unavailable".to_owned())?;
    state.set(token)
}

#[tauri::command]
async fn stronghold_available(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(app.try_state::<StrongholdAuthStore>().is_some())
}

fn legacy_secure_profile_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("tachyon-stronghold-auth-profile.json"))
}

async fn read_secure_profile(app: &tauri::AppHandle) -> Result<Option<SecureAuthProfile>, String> {
    let store = app
        .try_state::<StrongholdAuthStore>()
        .ok_or_else(|| "Stronghold auth backend is unavailable".to_owned())?;
    if let Some(bytes) = store.get_record(AUTH_PROFILE_RECORD)? {
        return serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("failed to decode Stronghold auth profile: {error}"));
    }

    let path = legacy_secure_profile_path(app)?;
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to read Stronghold auth profile `{}`: {error}",
                path.display()
            ));
        }
    };
    let profile: SecureAuthProfile = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to decode legacy auth profile: {error}"))?;
    store.insert_record(
        AUTH_PROFILE_RECORD,
        serde_json::to_vec(&profile).map_err(|error| {
            format!("failed to encode migrated Stronghold auth profile: {error}")
        })?,
    )?;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(Some(profile))
}

async fn write_secure_profile(
    app: &tauri::AppHandle,
    profile: &SecureAuthProfile,
) -> Result<(), String> {
    let store = app
        .try_state::<StrongholdAuthStore>()
        .ok_or_else(|| "Stronghold auth backend is unavailable".to_owned())?;
    let bytes = serde_json::to_vec(profile)
        .map_err(|error| format!("failed to encode Stronghold auth profile: {error}"))?;
    store.insert_record(AUTH_PROFILE_RECORD, bytes)
}

fn validate_traffic_config(config: &Value) -> ApiResponse {
    let traffic = match traffic_configuration_from_json(config) {
        Ok(traffic) => traffic,
        Err(message) => return failed(&message),
    };
    let gateways = traffic.gateways.len();
    let routes = traffic.routes.len();
    if gateways == 0 && routes == 0 {
        return failed("WIT validation failed: gateways or routes are required");
    }
    passed(format!(
        "WIT validation passed: {gateways} gateway(s), {routes} route(s)."
    ))
}

fn validate_resilience_config(config: &Value) -> ApiResponse {
    let _wit_config =
        wit_contracts::resilience::exports::tachyon::resilience_config::config_resilience::ResilienceConfiguration {
            policies: Vec::new(),
        };
    let policies = array_len(config, &["policies"]);
    if policies == 0 && !config.is_object() {
        return failed("Resilience validation failed: payload must be an object");
    }
    passed(format!(
        "Resilience WIT validation passed: {policies} policy item(s)."
    ))
}

fn validate_ai_config(config: &Value) -> ApiResponse {
    let wit_config = wit_contracts::ai::exports::tachyon::ai_config::config_ai::AiConfiguration {
        deployments: Vec::new(),
    };
    if let Some(kv_cache_size) = config.get("kv_cache_size").and_then(Value::as_u64) {
        if !(8..=128).contains(&kv_cache_size) {
            return failed("AI WIT validation failed: kv_cache_size must be between 8 and 128 GB");
        }
    }
    let deployments = array_len(config, &["deployments"]).max(wit_config.deployments.len());
    passed(format!(
        "AI WIT validation passed: {deployments} deployment(s)."
    ))
}

fn validate_security_identity_config(config: &Value) -> ApiResponse {
    let providers = array_len(config, &["providers"]);
    let authz = array_len(config, &["authz"]);
    let rate_limits = array_len(config, &["rate_limits", "rateLimits", "rate-limits"]);
    if providers == 0 && authz == 0 && rate_limits == 0 && !config.is_object() {
        return failed("Security WIT validation failed: payload must be an object");
    }
    passed(format!(
        "Security config accepted: providers={providers}, authz={authz}, rate_limits={rate_limits}."
    ))
}

fn validate_rbac_panel_config(config: &Value) -> ApiResponse {
    let roles = array_len(config, &["roles"]);
    let bindings = array_len(config, &["bindings"]);
    passed(format!(
        "RBAC WIT validation passed: roles={roles}, bindings={bindings}."
    ))
}

fn validate_workloads_panel_config(config: &Value) -> ApiResponse {
    let workloads = array_len(config, &["workloads"]);
    let secrets = array_len(config, &["secrets"]);
    passed(format!(
        "Workload WIT validation passed: workloads={workloads}, secret providers={secrets}."
    ))
}

fn validate_observability_panel_config(config: &Value) -> ApiResponse {
    let endpoint = nested_string(config, "telemetry.traces.otlp_endpoint")
        .or_else(|| nested_string(config, "telemetry.traces.otlp-endpoint"))
        .or_else(|| config.get("otlp_endpoint").and_then(Value::as_str))
        .map(str::to_owned);
    let _wit_config =
        wit_contracts::observability::exports::tachyon::observability_config::config_observability::OpsConfiguration {
            telemetry: wit_contracts::observability::exports::tachyon::observability_config::config_observability::TelemetryConfig {
                logs: wit_contracts::observability::exports::tachyon::observability_config::config_observability::LogPolicy {
                    global_level: wit_contracts::observability::exports::tachyon::observability_config::config_observability::LogLevel::Info,
                },
                traces: wit_contracts::observability::exports::tachyon::observability_config::config_observability::TracePolicy {
                    otlp_endpoint: endpoint.clone(),
                    sample_rate: config
                        .get("sample_rate")
                        .or_else(|| nested_value(config, "telemetry.traces.sample_rate"))
                        .and_then(Value::as_f64)
                        .unwrap_or(1.0),
                },
            },
            quotas: Vec::new(),
        };
    if let Some(endpoint) = endpoint.as_deref() {
        if !endpoint.is_empty()
            && !endpoint.starts_with("https://")
            && !endpoint.starts_with("http://")
        {
            return failed(
                "Observability WIT validation failed: otlp_endpoint must use http(s)://",
            );
        }
    }
    passed("Observability WIT validation passed.")
}

fn validate_storage_panel_config(config: &Value) -> ApiResponse {
    let _wit_config =
        wit_contracts::storage::exports::tachyon::storage_config::config_storage::StorageConfiguration {
            volumes: Vec::new(),
            s3_backends: Vec::new(),
            kv_partitions: Vec::new(),
        };
    let volumes = array_len(config, &["volumes"]);
    let s3 = array_len(config, &["s3_backends", "s3Backends", "s3-backends"]);
    let kv = array_len(config, &["kv_partitions", "kvPartitions", "kv-partitions"]);
    passed(format!(
        "Storage WIT validation passed: volumes={volumes}, s3_backends={s3}, kv_partitions={kv}."
    ))
}

fn validate_fleet_panel_config(config: &Value) -> ApiResponse {
    let wit_config =
        wit_contracts::fleet::exports::tachyon::fleet_config::config_fleet::FleetConfiguration {
            profiles: Vec::new(),
        };
    let profiles = array_len(config, &["profiles"]).max(wit_config.profiles.len());
    passed(format!(
        "Fleet WIT validation passed: {profiles} profile(s)."
    ))
}

fn validate_supply_chain_panel_config(config: &Value) -> ApiResponse {
    let bundles = array_len(config, &["bundles"]);
    if let Some(signature_key) = config.get("signature_key").and_then(Value::as_str) {
        if !signature_key.starts_with("sha256:") {
            return failed(
                "Supply Chain WIT validation failed: signature_key must start with sha256:",
            );
        }
    }
    passed(format!(
        "Supply chain WIT validation passed: {bundles} bundle(s)."
    ))
}

fn traffic_configuration_from_json(
    config: &Value,
) -> Result<
    wit_contracts::routing::exports::tachyon::routing::config_routing::TrafficConfiguration,
    String,
> {
    use wit_contracts::routing::exports::tachyon::routing::config_routing::{
        AccelMode, GatewayConfig, RouteConfig, TrafficConfiguration,
    };

    let gateway_values = array_values(config, &["gateways", "spec.gateways"]);
    let route_values = array_values(config, &["routes", "spec.routes"]);
    let gateways = gateway_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let name = string_field(value, &["name"], &format!("gateway-{index}"));
            let protocol = string_field(value, &["proto", "protocol"], "http");
            let bind_address = string_field(
                value,
                &["bind_address", "bindAddress", "bind-address"],
                "0.0.0.0:80",
            );
            Ok(GatewayConfig {
                name,
                proto: protocol_from_string(&protocol)?,
                bind_address,
                accel: AccelMode::Userspace,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let routes = route_values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let name = string_field(value, &["name"], &format!("route-{index}"));
            let gateway_refs =
                string_array_field(value, &["gateway_refs", "gatewayRefs", "gateway-refs"]);
            Ok(RouteConfig { name, gateway_refs })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(TrafficConfiguration { gateways, routes })
}

fn protocol_from_string(
    value: &str,
) -> Result<wit_contracts::routing::exports::tachyon::routing::config_routing::Protocol, String> {
    use wit_contracts::routing::exports::tachyon::routing::config_routing::Protocol;
    match value.trim().to_ascii_lowercase().as_str() {
        "tcp" => Ok(Protocol::Tcp),
        "udp" => Ok(Protocol::Udp),
        "http" => Ok(Protocol::Http),
        "https" => Ok(Protocol::Https),
        "grpc" => Ok(Protocol::Grpc),
        "uds" => Ok(Protocol::Uds),
        other => Err(format!(
            "WIT validation failed: unsupported routing protocol `{other}`"
        )),
    }
}

fn array_len(config: &Value, paths: &[&str]) -> usize {
    paths
        .iter()
        .find_map(|path| {
            nested_value(config, path)
                .and_then(Value::as_array)
                .map(Vec::len)
        })
        .unwrap_or(0)
}

fn array_values<'a>(config: &'a Value, paths: &[&str]) -> Vec<&'a Value> {
    paths
        .iter()
        .find_map(|path| nested_value(config, path).and_then(Value::as_array))
        .map(|values| values.iter().collect())
        .unwrap_or_default()
}

fn string_field(value: &Value, keys: &[&str], fallback: &str) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn string_array_field(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn nested_string<'a>(config: &'a Value, path: &str) -> Option<&'a str> {
    nested_value(config, path).and_then(Value::as_str)
}

fn nested_value<'a>(config: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(config, |value, segment| value.get(segment))
}

fn failed(message: &str) -> ApiResponse {
    ApiResponse {
        success: false,
        message: message.to_owned(),
    }
}

fn passed(message: impl Into<String>) -> ApiResponse {
    ApiResponse {
        success: true,
        message: message.into(),
    }
}

fn stronghold_profile_key(data_dir: &std::path::Path) -> Result<Vec<u8>, String> {
    let key_path = data_dir.join("stronghold-profile.key");
    match std::fs::read(&key_path) {
        Ok(bytes) if bytes.len() == STRONGHOLD_PROFILE_KEY_BYTES => Ok(bytes),
        Ok(_) => Err(format!(
            "Stronghold profile key `{}` has an invalid length",
            key_path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("failed to create Stronghold key dir: {error}"))?;
            }
            let mut key = vec![0_u8; STRONGHOLD_PROFILE_KEY_BYTES];
            OsRng.fill_bytes(&mut key);
            std::fs::write(&key_path, &key)
                .map_err(|error| format!("failed to write Stronghold profile key: {error}"))?;
            Ok(key)
        }
        Err(error) => Err(format!(
            "failed to read Stronghold profile key `{}`: {error}",
            key_path.display()
        )),
    }
}

fn main() {
    let result = tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|error| tauri::Error::Anyhow(error.into()))?;
            // Export the runtime workspace root so tachyon-client can resolve
            // integrity.lock without relying on the compile-time CARGO_MANIFEST_DIR.
            std::env::set_var("TACHYON_WORKSPACE_ROOT", &data_dir);
            let salt_path = data_dir.join("stronghold-salt.txt");
            let profile_key = stronghold_profile_key(&data_dir)
                .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
            let profile_store = StrongholdAuthStore::new(
                data_dir.join("tachyon-auth-profile.stronghold"),
                profile_key,
            )
            .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
            app.manage(profile_store);
            app.manage(MfaSessionState::default());
            app.handle()
                .plugin(tauri_plugin_stronghold::Builder::with_argon2(&salt_path).build())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_mesh_graph,
            get_topology_graph,
            get_metrics,
            tail_logs,
            get_shadow_diffs,
            connect_to_node,
            authn_login,
            finalize_login,
            validate_signup_token,
            stage_signup,
            finalize_signup,
            iam_list_users,
            iam_update_user,
            iam_delete_user,
            iam_list_groups,
            iam_upsert_group,
            iam_delete_group,
            fetch_user_audit_log,
            fetch_canary_status,
            abort_canary_rollout,
            iam_regen_mfa,
            generate_recovery_codes,
            regenerate_account_security,
            generate_pat,
            generate_operator_invite,
            push_asset,
            push_large_model,
            get_resources,
            get_hardware_status,
            list_enrolled_nodes,
            get_node_capabilities,
            list_registered_systems,
            list_deployed_systems,
            get_cluster_hardware_summary,
            get_staged_config,
            get_active_config,
            toggle_system_route,
            get_staged_system_routes,
            validate_hardware_policy,
            save_resource,
            delete_resource,
            apply_configuration,
            seal_and_apply_manifest,
            get_node_public_key,
            bundle_and_apply_manifest,
            list_s3_volumes,
            attach_s3_volume,
            detach_s3_volume,
            list_volume_backups,
            backup_volume,
            restore_volume,
            save_credentials,
            load_credentials,
            delete_credentials,
            save_custom_ca,
            load_custom_ca,
            clear_custom_ca,
            verify_session_totp,
            stronghold_available
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Tachyon UI encountered a fatal error: {error}");
        std::process::exit(1);
    }
}
