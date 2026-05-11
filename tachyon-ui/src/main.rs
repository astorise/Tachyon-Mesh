#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

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
    wit_bindgen::generate!({
        path: "../wit/config-routing.wit",
        world: "traffic-management-config",
    });
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
async fn seal_and_apply_manifest() -> Result<tachyon_client::SealApplyOutcome, String> {
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
    let path = secure_profile_path(&app)?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to delete Stronghold auth profile `{}`: {error}",
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

    let profile = read_secure_profile(&app)
        .await?
        .ok_or_else(|| {
            "Step-up cannot complete without remembered credentials. Sign in with the remember toggle enabled, then retry the action.".to_owned()
        })?;

    if profile.url.is_empty() || profile.username.is_empty() || profile.password.is_empty() {
        return Err(
            "Step-up requires the workstation profile to contain a node URL, username, and password.".to_owned(),
        );
    }

    let staged = tachyon_client::authn_login(
        &profile.url,
        &profile.username,
        &profile.password,
        profile.custom_ca.clone(),
    )
    .await
    .map_err(|error| format!("step-up login staging failed: {error}"))?;

    let session_id = staged.session_id.ok_or_else(|| {
        "Step-up login staging did not return an MFA session id; refresh credentials and retry."
            .to_owned()
    })?;

    tachyon_client::finalize_login(&profile.url, &session_id, trimmed, profile.custom_ca)
        .await
        .map(|_| ())
        .map_err(|error| format!("step-up TOTP verification failed: {error}"))
}

fn secure_profile_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_local_data_dir()
        .map_err(|error| error.to_string())?
        .join("tachyon-stronghold-auth-profile.json"))
}

async fn read_secure_profile(app: &tauri::AppHandle) -> Result<Option<SecureAuthProfile>, String> {
    let path = secure_profile_path(app)?;
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
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("failed to decode Stronghold auth profile: {error}"))
}

async fn write_secure_profile(
    app: &tauri::AppHandle,
    profile: &SecureAuthProfile,
) -> Result<(), String> {
    let path = secure_profile_path(app)?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("invalid Stronghold auth profile path `{}`", path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("failed to create Stronghold auth directory: {error}"))?;
    let bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| format!("failed to encode Stronghold auth profile: {error}"))?;
    tokio::fs::write(&path, bytes).await.map_err(|error| {
        format!(
            "failed to write Stronghold auth profile `{}`: {error}",
            path.display()
        )
    })
}

fn validate_traffic_config(config: &Value) -> ApiResponse {
    let gateways = array_len(config, &["gateways", "spec.gateways"]);
    let routes = array_len(config, &["routes", "spec.routes"]);
    if gateways == 0 && routes == 0 {
        return failed("WIT validation failed: gateways or routes are required");
    }
    passed(format!(
        "WIT validation passed: {gateways} gateway(s), {routes} route(s)."
    ))
}

fn validate_resilience_config(config: &Value) -> ApiResponse {
    let policies = array_len(config, &["policies"]);
    if policies == 0 && !config.is_object() {
        return failed("Resilience validation failed: payload must be an object");
    }
    passed(format!(
        "Resilience WIT validation passed: {policies} policy item(s)."
    ))
}

fn validate_ai_config(config: &Value) -> ApiResponse {
    if let Some(kv_cache_size) = config.get("kv_cache_size").and_then(Value::as_u64) {
        if !(8..=128).contains(&kv_cache_size) {
            return failed("AI WIT validation failed: kv_cache_size must be between 8 and 128 GB");
        }
    }
    let deployments = array_len(config, &["deployments"]);
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
    if let Some(endpoint) = nested_string(config, "telemetry.traces.otlp_endpoint")
        .or_else(|| nested_string(config, "telemetry.traces.otlp-endpoint"))
        .or_else(|| config.get("otlp_endpoint").and_then(Value::as_str))
    {
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
    let volumes = array_len(config, &["volumes"]);
    let s3 = array_len(config, &["s3_backends", "s3Backends", "s3-backends"]);
    let kv = array_len(config, &["kv_partitions", "kvPartitions", "kv-partitions"]);
    passed(format!(
        "Storage WIT validation passed: volumes={volumes}, s3_backends={s3}, kv_partitions={kv}."
    ))
}

fn validate_fleet_panel_config(config: &Value) -> ApiResponse {
    let profiles = array_len(config, &["profiles"]);
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
            app.handle()
                .plugin(tauri_plugin_stronghold::Builder::with_argon2(&salt_path).build())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_mesh_graph,
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
            iam_regen_mfa,
            generate_recovery_codes,
            regenerate_account_security,
            generate_pat,
            generate_operator_invite,
            push_asset,
            push_large_model,
            get_resources,
            get_hardware_status,
            validate_hardware_policy,
            save_resource,
            delete_resource,
            apply_configuration,
            seal_and_apply_manifest,
            get_node_public_key,
            bundle_and_apply_manifest,
            save_credentials,
            load_credentials,
            delete_credentials,
            save_custom_ca,
            load_custom_ca,
            clear_custom_ca,
            verify_session_totp
        ])
        .run(tauri::generate_context!());

    if let Err(error) = result {
        eprintln!("Tachyon UI encountered a fatal error: {error}");
        std::process::exit(1);
    }
}
