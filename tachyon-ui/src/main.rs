#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::Deserialize;
use serde_json::Value;
use tauri::Emitter;

#[derive(serde::Serialize)]
struct ApiResponse {
    success: bool,
    message: String,
}

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
async fn apply_configuration(domain: String, payload: Value) -> Result<ApiResponse, String> {
    println!("Received IPC Intent for Domain: {domain}");

    if domain == "config-routing" {
        if payload.get("api_version").and_then(Value::as_str) == Some("routing.tachyon.io/v1alpha1")
        {
            return Ok(ApiResponse {
                success: true,
                message: "Configuration dynamically validated by Rust Backend".to_owned(),
            });
        }

        return Err("Invalid Schema: Missing or incorrect api_version".to_owned());
    }

    Err(format!("Unknown configuration domain: {domain}"))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_mesh_graph,
            connect_to_node,
            authn_login,
            validate_signup_token,
            stage_signup,
            finalize_signup,
            iam_list_users,
            iam_regen_mfa,
            generate_recovery_codes,
            regenerate_account_security,
            generate_pat,
            push_asset,
            push_large_model,
            get_resources,
            get_hardware_status,
            validate_hardware_policy,
            save_resource,
            delete_resource,
            apply_configuration
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
