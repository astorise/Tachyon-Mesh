#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::Deserialize;
use tauri::Emitter;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthnLoginPayload {
    url: String,
    username: String,
    password: String,
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_mesh_graph,
            connect_to_node,
            authn_login,
            iam_list_users,
            iam_regen_mfa,
            generate_recovery_codes,
            regenerate_account_security,
            generate_pat,
            push_asset,
            push_large_model
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
