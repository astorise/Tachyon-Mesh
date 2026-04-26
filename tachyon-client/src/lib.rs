use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    net::IpAddr,
    path::PathBuf,
    sync::{OnceLock, RwLock},
};
use tokio::io::AsyncReadExt;

const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_RECOVERY_CODES_PATH: &str = "/admin/security/recovery-codes";
const ADMIN_ACCOUNT_SECURITY_PATH: &str = "/admin/security/2fa/regenerate";
const ADMIN_PAT_PATH: &str = "/admin/security/pats";
const ADMIN_ASSET_UPLOAD_PATH: &str = "/admin/assets";
const AUTH_SIGNUP_VALIDATE_PATH: &str = "/auth/signup/validate-token";
const AUTH_SIGNUP_STAGE_PATH: &str = "/auth/signup/stage";
const AUTH_SIGNUP_FINALIZE_PATH: &str = "/auth/signup/finalize";
const EXPECTED_HASH_HEADER: &str = "x-tachyon-expected-sha256";
const ADMIN_MODEL_INIT_PATH: &str = "/admin/models/init";
const ADMIN_MODEL_UPLOAD_PATH: &str = "/admin/models/upload";
const ADMIN_MODEL_COMMIT_PATH: &str = "/admin/models/commit";
const MODEL_CHUNK_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct InstanceConfig {
    pub url: String,
    pub token: String,
    pub mtls_cert: Option<Vec<u8>>,
    pub mtls_key: Option<Vec<u8>>,
}

static CONNECTION_STATE: OnceLock<RwLock<Option<InstanceConfig>>> = OnceLock::new();
static SESSION_OPERATOR: OnceLock<RwLock<Option<String>>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthLoginResponse {
    pub username: String,
    pub endpoint: String,
    pub requires_mfa: bool,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IamUserSummary {
    pub username: String,
    pub groups: Vec<String>,
    pub security_status: String,
}

#[derive(Debug, Deserialize)]
struct IntegrityManifest {
    config_payload: String,
}

#[derive(Debug, Deserialize)]
struct SealedConfig {
    #[serde(default)]
    routes: Vec<serde_json::Value>,
    #[serde(default)]
    batch_targets: Vec<serde_json::Value>,
    #[serde(default)]
    resources: std::collections::BTreeMap<String, serde_json::Value>,
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

#[derive(Debug, Deserialize)]
struct AssetUploadResponse {
    asset_uri: String,
}

#[derive(Debug, Serialize)]
struct InitModelUploadRequest<'a> {
    expected_hash: &'a str,
    size_bytes: u64,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshRouteSummary {
    pub path: String,
    pub name: String,
    pub role: String,
    pub target_count: usize,
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

fn connection_state() -> &'static RwLock<Option<InstanceConfig>> {
    CONNECTION_STATE.get_or_init(|| RwLock::new(None))
}

fn operator_state() -> &'static RwLock<Option<String>> {
    SESSION_OPERATOR.get_or_init(|| RwLock::new(None))
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tachyon-client should live directly under the workspace root")
        .to_path_buf()
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

    let raw_lockfile = read_lockfile().await?;
    parse_engine_status(&raw_lockfile)
}

pub async fn get_mesh_graph() -> Result<MeshGraphSnapshot> {
    let raw_lockfile = read_lockfile().await?;
    let manifest: IntegrityManifest =
        serde_json::from_str(&raw_lockfile).context("failed to parse integrity.lock manifest")?;
    let sealed_config: SealedConfig = serde_json::from_str(&manifest.config_payload)
        .context("failed to parse sealed config payload")?;
    let current = current_connection();
    let source = current
        .as_ref()
        .map(|config| config.url.clone())
        .unwrap_or_else(|| "workspace://local".to_owned());
    let status = if let Some(config) = current.as_ref() {
        match fetch_remote_status(config).await {
            Ok(status) => status,
            Err(_) => parse_engine_status(&raw_lockfile)?,
        }
    } else {
        parse_engine_status(&raw_lockfile)?
    };

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

const OVERLAY_FILE_NAME: &str = "tachyon.resources.json";

fn overlay_path() -> PathBuf {
    workspace_root().join(OVERLAY_FILE_NAME)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ResourceOverlayFile {
    #[serde(default)]
    resources: Vec<MeshResource>,
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
    set_connection(url.trim().to_owned(), password.trim().to_owned(), cert)
        .await
        .map_err(anyhow::Error::msg)?;

    let mut state = operator_state()
        .write()
        .expect("operator state should not be poisoned");
    *state = Some(normalized_username.clone());

    Ok(AuthLoginResponse {
        username: normalized_username,
        endpoint: current_connection()
            .map(|config| config.url)
            .unwrap_or_else(|| url.trim().to_owned()),
        requires_mfa: true,
    })
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
    })
}

pub async fn iam_list_users() -> Result<Vec<IamUserSummary>> {
    let _config = require_connection()?;
    let username = current_operator_name().unwrap_or_else(|| "admin".to_owned());

    Ok(vec![IamUserSummary {
        username,
        groups: vec!["admin".to_owned(), "ops".to_owned()],
        security_status: "Recovery bundle managed through desktop onboarding".to_owned(),
    }])
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

pub async fn push_large_model(file_path: &str) -> Result<String> {
    push_large_model_with_progress(file_path, |_| {}).await
}

pub async fn push_large_model_with_progress<F>(file_path: &str, mut progress: F) -> Result<String>
where
    F: FnMut(f32) + Send,
{
    let size_bytes = tokio::fs::metadata(file_path)
        .await
        .with_context(|| format!("failed to read metadata for model `{file_path}`"))?
        .len();
    if size_bytes == 0 {
        anyhow::bail!("model payload `{file_path}` must not be empty");
    }

    let expected_hash = hash_file_sha256(file_path).await?;
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let init_response = client
        .post(build_endpoint_url(&config.url, ADMIN_MODEL_INIT_PATH)?)
        .bearer_auth(&config.token)
        .json(&InitModelUploadRequest {
            expected_hash: &expected_hash,
            size_bytes,
        })
        .send()
        .await
        .with_context(|| format!("failed to initialize model upload for `{file_path}`"))?;
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

    let mut file = tokio::fs::File::open(file_path)
        .await
        .with_context(|| format!("failed to open model `{file_path}` for upload"))?;
    let mut buffer = vec![0_u8; MODEL_CHUNK_BYTES];
    let mut uploaded_bytes = 0_u64;
    let mut part = 1_u64;

    loop {
        let read = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to read model `{file_path}`"))?;
        if read == 0 {
            break;
        }

        let upload_url = format!(
            "{}/{}?part={part}",
            build_endpoint_url(&config.url, ADMIN_MODEL_UPLOAD_PATH)?,
            init_payload.upload_id
        );
        let response = client
            .put(upload_url)
            .bearer_auth(&config.token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(buffer[..read].to_vec())
            .send()
            .await
            .with_context(|| format!("failed to upload chunk {part} for model `{file_path}`"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "failed to read upload error body".to_owned());
            anyhow::bail!("model chunk upload failed with {status}: {}", body.trim());
        }

        uploaded_bytes = uploaded_bytes.saturating_add(read as u64);
        progress(((uploaded_bytes as f64 / size_bytes as f64) * 100.0) as f32);
        part = part.saturating_add(1);
    }

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
        .with_context(|| format!("failed to commit model upload for `{file_path}`"))?;
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

    progress(100.0);
    let payload: CommitModelUploadResponse = serde_json::from_slice(&commit_body)
        .context("failed to decode model-commit response payload")?;
    Ok(payload.model_path)
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

    Some(MeshRouteSummary {
        path,
        name,
        role,
        target_count,
    })
}

fn batch_target_name(batch_target: &serde_json::Value) -> Option<String> {
    batch_target
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
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

fn build_http_client(config: &InstanceConfig) -> Result<reqwest::Client> {
    let url = reqwest::Url::parse(&config.url)
        .with_context(|| format!("invalid Tachyon node URL `{}`", config.url))?;
    let mut builder = reqwest::Client::builder();

    if allows_insecure_local_tls(&url) {
        builder = builder.danger_accept_invalid_certs(true);
    }

    if let Some(identity_bytes) = config.mtls_cert.as_deref() {
        builder = builder.identity(parse_identity(identity_bytes)?);
    }

    builder
        .build()
        .context("failed to build authenticated HTTP client")
}

fn parse_identity(identity_bytes: &[u8]) -> Result<reqwest::Identity> {
    reqwest::Identity::from_pkcs8_pem(identity_bytes, identity_bytes)
        .or_else(|_| reqwest::Identity::from_pkcs12_der(identity_bytes, ""))
        .context("failed to parse mTLS identity bundle as PEM or PKCS#12")
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

async fn hash_file_sha256(file_path: &str) -> Result<String> {
    let mut file = tokio::fs::File::open(file_path)
        .await
        .with_context(|| format!("failed to open model `{file_path}` for hashing"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; MODEL_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .with_context(|| format!("failed to hash model `{file_path}`"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_engine_status_summarizes_routes_and_batches() {
        let raw_lockfile = r#"{
          "config_payload": "{\"routes\":[{\"path\":\"/api/a\"},{\"path\":\"/api/b\"}],\"batch_targets\":[{\"name\":\"gc-job\"}]}"
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
