#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod app_config;
mod app_logging;

use app_config::{AppConfig, LogLevel};
use app_logging::{AppLogger, FrontendLogPayload};
use rand::{rngs::SysRng, TryRng};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tauri_plugin_stronghold::stronghold::Stronghold;

const STRONGHOLD_PROFILE_CLIENT: &[u8] = b"tachyon-ui-auth";
const AUTH_PROFILE_RECORD: &[u8] = b"auth_profile";
const STRONGHOLD_PROFILE_KEY_BYTES: usize = 32;
const MFA_SESSION_TTL_SECONDS: u64 = 20 * 60;
const UPLOAD_STATUS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

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
fn recommend_concurrency_policy(
    pattern: String,
    writes_shared_state: Option<bool>,
    requires_ordering: Option<bool>,
    max_latency_ms: Option<u64>,
) -> tachyon_client::ConcurrencyRecommendation {
    tachyon_client::recommend_concurrency_policy(
        &pattern,
        &tachyon_client::ConcurrencyRequirements {
            writes_shared_state: writes_shared_state.unwrap_or(false),
            requires_ordering: requires_ordering.unwrap_or(false),
            max_latency_ms,
        },
    )
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
async fn get_manifest_config() -> Result<serde_json::Value, String> {
    tachyon_client::get_manifest_config()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn apply_manifest_config(
    config: serde_json::Value,
) -> Result<tachyon_client::SealApplyOutcome, String> {
    tachyon_client::apply_manifest_config(config)
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
async fn import_faas_package(
    bytes: Vec<u8>,
) -> Result<tachyon_client::ImportPackageResult, String> {
    tachyon_client::import_faas_package_bytes(&bytes)
        .await
        .map_err(|error| error.to_string())
}

/// Open a native folder-open dialog and return the chosen model directory path,
/// or `None` if the operator cancelled. A complete model is a directory (weights
/// plus `tokenizer.json`, and `config.json` for safetensors); the client tars
/// and gzips it on the fly during upload. Implemented as an app command (no
/// plugin capability needed) wrapping the dialog plugin's Rust API.
#[tauri::command]
async fn pick_model_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Select a model folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|error| error.to_string())?;
    Ok(picked.map(|path| path.to_string()))
}

#[tauri::command]
async fn push_large_model(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let mut last_phase = String::new();
    let mut last_percent = i32::MIN;
    let mut last_emit = None;
    tachyon_client::push_large_model_with_status(&path, |status| {
        let now = Instant::now();
        if should_emit_upload_status(&status, &last_phase, last_percent, last_emit, now) {
            let phase = status.phase.clone();
            let percent = rounded_upload_percent(status.percentage);
            let _ = app.emit("upload_progress", status.percentage);
            let _ = app.emit("upload_status", status);
            last_phase = phase;
            last_percent = percent;
            last_emit = Some(now);
        }
    })
    .await
    .map_err(|error| error.to_string())
}

fn should_emit_upload_status(
    status: &tachyon_client::ModelUploadStatus,
    last_phase: &str,
    last_percent: i32,
    last_emit: Option<Instant>,
    now: Instant,
) -> bool {
    if status.phase != last_phase {
        return true;
    }
    let percent = rounded_upload_percent(status.percentage);
    if percent != last_percent {
        return true;
    }
    last_emit
        .map(|emitted_at| now.saturating_duration_since(emitted_at) >= UPLOAD_STATUS_EMIT_INTERVAL)
        .unwrap_or(true)
}

fn rounded_upload_percent(percentage: f32) -> i32 {
    percentage.clamp(0.0, 100.0).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload_status(phase: &str, percentage: f32) -> tachyon_client::ModelUploadStatus {
        tachyon_client::ModelUploadStatus {
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

    #[test]
    fn upload_status_emits_on_phase_change() {
        let now = Instant::now();
        assert!(should_emit_upload_status(
            &upload_status("uploading", 10.0),
            "preparing",
            10,
            Some(now),
            now
        ));
    }

    #[test]
    fn upload_status_emits_on_rounded_percent_change() {
        let now = Instant::now();
        assert!(should_emit_upload_status(
            &upload_status("uploading", 11.0),
            "uploading",
            10,
            Some(now),
            now
        ));
    }

    #[test]
    fn upload_status_suppresses_duplicate_updates_inside_interval() {
        let now = Instant::now();
        assert!(!should_emit_upload_status(
            &upload_status("uploading", 10.2),
            "uploading",
            10,
            Some(now),
            now + Duration::from_millis(50)
        ));
    }

    #[test]
    fn upload_status_emits_duplicate_updates_after_interval() {
        let now = Instant::now();
        assert!(should_emit_upload_status(
            &upload_status("uploading", 10.2),
            "uploading",
            10,
            Some(now),
            now + UPLOAD_STATUS_EMIT_INTERVAL
        ));
    }
}

#[tauri::command]
async fn preview_large_model_upload(
    path: String,
) -> Result<tachyon_client::ModelUploadPlanPreview, String> {
    tachyon_client::preview_large_model_upload(&path)
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
async fn list_available_models() -> Result<Vec<tachyon_client::AvailableAiModel>, String> {
    tachyon_client::list_available_ai_models()
        .await
        .map_err(|error| error.to_string())
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
async fn get_cluster_features() -> Result<tachyon_client::ClusterFeatureSet, String> {
    tachyon_client::get_cluster_features()
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
            SysRng
                .try_fill_bytes(&mut key)
                .map_err(|error| format!("failed to generate Stronghold profile key: {error}"))?;
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

#[tauri::command]
fn log_frontend_event(
    logger: tauri::State<'_, AppLogger>,
    payload: FrontendLogPayload,
) -> Result<(), String> {
    logger.write_frontend(payload)
}

fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|error| tauri::Error::Anyhow(error.into()))?;
            let config = AppConfig::load_or_create(&data_dir)
                .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
            let logger = AppLogger::new(&data_dir, &config.logging)
                .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
            logger.register_global();
            logger
                .write(
                    LogLevel::Info,
                    "application.startup",
                    "Tachyon UI is starting",
                )
                .map_err(|error| tauri::Error::Anyhow(std::io::Error::other(error).into()))?;
            app.manage(logger);
            app.manage(config);
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
            import_faas_package,
            pick_model_file,
            preview_large_model_upload,
            push_large_model,
            get_resources,
            get_hardware_status,
            list_available_models,
            list_enrolled_nodes,
            get_node_capabilities,
            list_registered_systems,
            list_deployed_systems,
            get_cluster_features,
            get_cluster_hardware_summary,
            get_staged_config,
            toggle_system_route,
            get_staged_system_routes,
            validate_hardware_policy,
            save_resource,
            delete_resource,
            seal_and_apply_manifest,
            get_node_public_key,
            bundle_and_apply_manifest,
            list_s3_volumes,
            attach_s3_volume,
            detach_s3_volume,
            list_volume_backups,
            backup_volume,
            restore_volume,
            recommend_concurrency_policy,
            get_manifest_config,
            apply_manifest_config,
            save_credentials,
            load_credentials,
            delete_credentials,
            save_custom_ca,
            load_custom_ca,
            clear_custom_ca,
            verify_session_totp,
            stronghold_available,
            log_frontend_event
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        app_logging::write_global(LogLevel::Error, "application.fatal", &error.to_string());
        eprintln!("Tachyon UI encountered a fatal error: {error}");
        std::process::exit(1);
    }
}
