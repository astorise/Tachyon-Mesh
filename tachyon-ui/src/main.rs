#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Emitter;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficConfig {
    api_version: String,
    kind: String,
    metadata: TrafficMetadata,
    spec: TrafficSpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficMetadata {
    name: String,
    environment: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficSpec {
    gateways: Vec<TrafficGateway>,
    routes: Vec<TrafficRoute>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficGateway {
    name: String,
    protocol: TrafficProtocol,
    bind_address: String,
}

#[derive(Debug, Deserialize)]
enum TrafficProtocol {
    #[serde(rename = "HTTP")]
    Http,
    #[serde(rename = "HTTPS")]
    Https,
    #[serde(rename = "TCP")]
    Tcp,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficRoute {
    name: String,
    gateway_refs: Vec<String>,
    #[serde(rename = "type")]
    route_type: TrafficRouteType,
    rules: Vec<TrafficRouteRule>,
}

#[derive(Debug, Deserialize)]
enum TrafficRouteType {
    #[serde(rename = "HTTP")]
    Http,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficRouteRule {
    #[serde(rename = "match")]
    match_rule: TrafficRuleMatch,
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficRuleMatch {
    path: TrafficPathMatch,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrafficPathMatch {
    prefix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResilienceConfig {
    timeout_ms: u64,
    retry_count: u32,
    circuit_breaker_threshold: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AiConfig {
    lora_mode: LoraMode,
    kv_cache_size: u32,
    tde_key: String,
    accelerator: Option<Accelerator>,
    xdp_offload: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecurityIdentityConfig {
    jwt_issuer: String,
    crdt_quota: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RbacPanelConfig {
    role: String,
    policy: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkloadsPanelConfig {
    engine: WorkloadEngine,
    secret_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WorkloadEngine {
    Wasmtime,
    Smolvm,
    Legacy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ObservabilityPanelConfig {
    otlp_endpoint: String,
    log_level: LogLevel,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoragePanelConfig {
    mount_path: String,
    s3_endpoint: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FleetPanelConfig {
    selector_tags: String,
    node_profile: NodeProfile,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum NodeProfile {
    EdgeLight,
    CoreHeavy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupplyChainPanelConfig {
    signature_key: String,
    air_gapped: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LoraMode {
    Dynamic,
    Static,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Accelerator {
    Npu,
    Tpu,
    Gpu,
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
    match domain.as_str() {
        "config-routing" | "routing" => match serde_json::from_value::<TrafficConfig>(payload) {
            Ok(config) => Ok(validate_traffic_config(config)),
            Err(error) => Ok(ApiResponse {
                success: false,
                message: format!("WIT validation failed: {error}"),
            }),
        },
        "config-resilience" | "resilience" => {
            match serde_json::from_value::<ResilienceConfig>(payload) {
                Ok(config) => Ok(validate_resilience_config(config)),
                Err(error) => Ok(ApiResponse {
                    success: false,
                    message: format!("Resilience validation failed: {error}"),
                }),
            }
        }
        "config-ai" | "ai_orchestration" => match serde_json::from_value::<AiConfig>(payload) {
            Ok(config) => Ok(validate_ai_config(config)),
            Err(error) => Ok(ApiResponse {
                success: false,
                message: format!("AI WIT validation failed: {error}"),
            }),
        },
        "config-security" | "identity" | "security-identity" => {
            match serde_json::from_value::<SecurityIdentityConfig>(payload) {
                Ok(config) => Ok(validate_security_identity_config(config)),
                Err(error) => Ok(ApiResponse {
                    success: false,
                    message: format!("Security WIT validation failed: {error}"),
                }),
            }
        }
        "config-rbac" | "rbac" => match serde_json::from_value::<RbacPanelConfig>(payload) {
            Ok(config) => Ok(validate_rbac_panel_config(config)),
            Err(error) => Ok(ApiResponse {
                success: false,
                message: format!("RBAC WIT validation failed: {error}"),
            }),
        },
        "config-workloads" | "workloads" => {
            match serde_json::from_value::<WorkloadsPanelConfig>(payload) {
                Ok(config) => Ok(validate_workloads_panel_config(config)),
                Err(error) => Ok(ApiResponse {
                    success: false,
                    message: format!("Workloads WIT validation failed: {error}"),
                }),
            }
        }
        "config-observability" | "observability" => {
            match serde_json::from_value::<ObservabilityPanelConfig>(payload) {
                Ok(config) => Ok(validate_observability_panel_config(config)),
                Err(error) => Ok(ApiResponse {
                    success: false,
                    message: format!("Observability WIT validation failed: {error}"),
                }),
            }
        }
        "config-storage" | "storage" => match serde_json::from_value::<StoragePanelConfig>(payload)
        {
            Ok(config) => Ok(validate_storage_panel_config(config)),
            Err(error) => Ok(ApiResponse {
                success: false,
                message: format!("Storage WIT validation failed: {error}"),
            }),
        },
        "config-fleet" | "fleet" => match serde_json::from_value::<FleetPanelConfig>(payload) {
            Ok(config) => Ok(validate_fleet_panel_config(config)),
            Err(error) => Ok(ApiResponse {
                success: false,
                message: format!("Fleet WIT validation failed: {error}"),
            }),
        },
        "config-assets" | "supply_chain" | "supply-chain" => {
            match serde_json::from_value::<SupplyChainPanelConfig>(payload) {
                Ok(config) => Ok(validate_supply_chain_panel_config(config)),
                Err(error) => Ok(ApiResponse {
                    success: false,
                    message: format!("Supply Chain WIT validation failed: {error}"),
                }),
            }
        }
        _ => Err(format!("Unknown configuration domain: {domain}")),
    }
}

fn validate_traffic_config(config: TrafficConfig) -> ApiResponse {
    if config.api_version != "routing.tachyon.io/v1alpha1" {
        return ApiResponse {
            success: false,
            message: format!(
                "WIT validation failed: unsupported api_version {}",
                config.api_version
            ),
        };
    }
    if config.kind != "TrafficConfiguration" {
        return ApiResponse {
            success: false,
            message: format!("WIT validation failed: unsupported kind {}", config.kind),
        };
    }
    if config.metadata.name.trim().is_empty() || config.metadata.environment.trim().is_empty() {
        return ApiResponse {
            success: false,
            message: "WIT validation failed: metadata.name and metadata.environment are required"
                .to_string(),
        };
    }

    for gateway in &config.spec.gateways {
        if gateway.name.trim().is_empty() || gateway.bind_address.trim().is_empty() {
            return ApiResponse {
                success: false,
                message:
                    "WIT validation failed: gateway.name and gateway.bind_address are required"
                        .to_string(),
            };
        }
        match gateway.protocol {
            TrafficProtocol::Http | TrafficProtocol::Https | TrafficProtocol::Tcp => {}
        }
    }

    for route in &config.spec.routes {
        if route.name.trim().is_empty() || route.gateway_refs.is_empty() || route.rules.is_empty() {
            return ApiResponse {
                success: false,
                message: "WIT validation failed: route.name, gateway_refs, and rules are required"
                    .to_string(),
            };
        }
        match route.route_type {
            TrafficRouteType::Http => {}
        }
        for rule in &route.rules {
            if rule.match_rule.path.prefix.trim().is_empty() || rule.target.trim().is_empty() {
                return ApiResponse {
                    success: false,
                    message: "WIT validation failed: rule path prefix and target are required"
                        .to_string(),
                };
            }
        }
    }

    ApiResponse {
        success: true,
        message: format!(
            "WIT validation passed for {}: {} gateway(s), {} route(s).",
            config.metadata.name,
            config.spec.gateways.len(),
            config.spec.routes.len()
        ),
    }
}

fn validate_resilience_config(config: ResilienceConfig) -> ApiResponse {
    if config.timeout_ms == 0 {
        return ApiResponse {
            success: false,
            message: "Resilience validation failed: timeout_ms must be greater than zero"
                .to_string(),
        };
    }
    if config.circuit_breaker_threshold == 0 {
        return ApiResponse {
            success: false,
            message:
                "Resilience validation failed: circuit_breaker_threshold must be greater than zero"
                    .to_string(),
        };
    }

    ApiResponse {
        success: true,
        message: format!(
            "Resilience validation passed: timeout={}ms, retries={}, breaker_threshold={}.",
            config.timeout_ms, config.retry_count, config.circuit_breaker_threshold
        ),
    }
}

fn validate_ai_config(config: AiConfig) -> ApiResponse {
    if !(8..=128).contains(&config.kv_cache_size) {
        return ApiResponse {
            success: false,
            message: "AI WIT validation failed: kv_cache_size must be between 8 and 128 GB"
                .to_string(),
        };
    }
    if config.tde_key.trim().is_empty() {
        return ApiResponse {
            success: false,
            message: "AI WIT validation failed: tde_key is required".to_string(),
        };
    }

    let lora_mode = match config.lora_mode {
        LoraMode::Dynamic => "dynamic",
        LoraMode::Static => "static",
    };
    let accelerator = match config.accelerator {
        Some(Accelerator::Npu) => "npu",
        Some(Accelerator::Tpu) => "tpu",
        Some(Accelerator::Gpu) => "gpu",
        None => "auto",
    };
    let xdp_status = if config.xdp_offload.unwrap_or(false) {
        "enabled"
    } else {
        "disabled"
    };

    ApiResponse {
        success: true,
        message: format!(
            "AI WIT validation passed: lora_mode={lora_mode}, kv_cache={}GB, accelerator={accelerator}, xdp_offload={xdp_status}.",
            config.kv_cache_size
        ),
    }
}

fn validate_security_identity_config(config: SecurityIdentityConfig) -> ApiResponse {
    if !config.jwt_issuer.starts_with("https://") {
        return ApiResponse {
            success: false,
            message: "Security WIT validation failed: jwt_issuer must use https://".to_owned(),
        };
    }
    if config.crdt_quota == 0 {
        return ApiResponse {
            success: false,
            message: "Security WIT validation failed: crdt_quota must be greater than zero"
                .to_owned(),
        };
    }
    ApiResponse {
        success: true,
        message: format!(
            "Identity security config accepted for issuer {} with {} req/sec CRDT quota",
            config.jwt_issuer, config.crdt_quota
        ),
    }
}

fn validate_rbac_panel_config(config: RbacPanelConfig) -> ApiResponse {
    if config.role.trim().is_empty() {
        return ApiResponse {
            success: false,
            message: "RBAC WIT validation failed: role is required".to_owned(),
        };
    }
    if !config.policy.is_object() {
        return ApiResponse {
            success: false,
            message: "RBAC WIT validation failed: policy must be a JSON object".to_owned(),
        };
    }
    ApiResponse {
        success: true,
        message: format!("RBAC policy accepted for role {}", config.role),
    }
}

fn validate_workloads_panel_config(config: WorkloadsPanelConfig) -> ApiResponse {
    if !config.secret_ref.is_empty() && !config.secret_ref.starts_with("vault://") {
        return ApiResponse {
            success: false,
            message: "Workloads WIT validation failed: secret_ref must use vault://".to_owned(),
        };
    }
    let engine = match config.engine {
        WorkloadEngine::Wasmtime => "wasmtime",
        WorkloadEngine::Smolvm => "smolvm",
        WorkloadEngine::Legacy => "legacy",
    };
    ApiResponse {
        success: true,
        message: format!("Workload contract accepted for {engine} runtime"),
    }
}

fn validate_observability_panel_config(config: ObservabilityPanelConfig) -> ApiResponse {
    if !config.otlp_endpoint.is_empty()
        && !config.otlp_endpoint.starts_with("https://")
        && !config.otlp_endpoint.starts_with("http://")
    {
        return ApiResponse {
            success: false,
            message: "Observability WIT validation failed: otlp_endpoint must use http(s)://"
                .to_owned(),
        };
    }
    let level = match config.log_level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    };
    let target = if config.otlp_endpoint.is_empty() {
        "local logging".to_owned()
    } else {
        config.otlp_endpoint
    };
    ApiResponse {
        success: true,
        message: format!("Observability config accepted at {level} level for {target}"),
    }
}

fn validate_storage_panel_config(config: StoragePanelConfig) -> ApiResponse {
    if !config.mount_path.starts_with('/') {
        return ApiResponse {
            success: false,
            message: "Storage WIT validation failed: mount_path must be absolute".to_owned(),
        };
    }
    if !config.s3_endpoint.is_empty()
        && !config.s3_endpoint.starts_with("https://")
        && !config.s3_endpoint.starts_with("http://")
    {
        return ApiResponse {
            success: false,
            message: "Storage WIT validation failed: s3_endpoint must use http(s)://".to_owned(),
        };
    }
    ApiResponse {
        success: true,
        message: format!("Storage config accepted for mount {}", config.mount_path),
    }
}

fn validate_fleet_panel_config(config: FleetPanelConfig) -> ApiResponse {
    if config.selector_tags.trim().is_empty() {
        return ApiResponse {
            success: false,
            message: "Fleet WIT validation failed: selector_tags are required".to_owned(),
        };
    }
    let profile = match config.node_profile {
        NodeProfile::EdgeLight => "edge-light",
        NodeProfile::CoreHeavy => "core-heavy",
    };
    ApiResponse {
        success: true,
        message: format!(
            "Fleet config accepted for profile {profile} with selectors {}",
            config.selector_tags
        ),
    }
}

fn validate_supply_chain_panel_config(config: SupplyChainPanelConfig) -> ApiResponse {
    if !config.signature_key.starts_with("sha256:") {
        return ApiResponse {
            success: false,
            message: "Supply Chain WIT validation failed: signature_key must start with sha256:"
                .to_owned(),
        };
    }
    let mode = if config.air_gapped {
        "air-gapped"
    } else {
        "connected"
    };
    ApiResponse {
        success: true,
        message: format!("Supply chain config accepted for {mode} mode"),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_mesh_graph,
            connect_to_node,
            authn_login,
            finalize_login,
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
