use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{OnceLock, RwLock},
};
use tokio::io::AsyncReadExt;

const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_RECOVERY_CODES_PATH: &str = "/admin/security/recovery-codes";
const ADMIN_ACCOUNT_SECURITY_PATH: &str = "/admin/security/2fa/regenerate";
const ADMIN_PAT_PATH: &str = "/admin/security/pats";
const ADMIN_ASSET_UPLOAD_PATH: &str = "/admin/assets";
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

fn connection_state() -> &'static RwLock<Option<InstanceConfig>> {
    CONNECTION_STATE.get_or_init(|| RwLock::new(None))
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

pub async fn generate_recovery_codes(username: &str) -> Result<Vec<String>> {
    let config = require_connection()?;
    let client = build_http_client(&config)?;
    let response = client
        .post(build_admin_url(&config.url, ADMIN_RECOVERY_CODES_PATH)?)
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
        .post(build_admin_url(&config.url, ADMIN_ACCOUNT_SECURITY_PATH)?)
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
        .post(build_admin_url(&config.url, ADMIN_PAT_PATH)?)
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
        .post(build_admin_url(&config.url, ADMIN_ASSET_UPLOAD_PATH)?)
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
        .post(build_admin_url(&config.url, ADMIN_MODEL_INIT_PATH)?)
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
            build_admin_url(&config.url, ADMIN_MODEL_UPLOAD_PATH)?,
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
        build_admin_url(&config.url, ADMIN_MODEL_COMMIT_PATH)?,
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
        .get(build_admin_url(&config.url, ADMIN_STATUS_PATH)?)
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

fn build_admin_url(base_url: &str, path: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(base_url)
        .with_context(|| format!("invalid Tachyon node URL `{base_url}`"))?;
    url.set_path(path);
    url.set_query(None);
    Ok(url.to_string())
}

fn allows_insecure_local_tls(url: &reqwest::Url) -> bool {
    matches!(
        url.host_str(),
        Some("127.0.0.1") | Some("localhost") | Some("::1")
    ) || url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("home-lab-k3s.wsl") || host.ends_with(".wsl"))
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
        let url = build_admin_url("https://127.0.0.1:4000/ui?stale=1", "/admin/status")
            .expect("admin URL should build");

        assert_eq!(url, "https://127.0.0.1:4000/admin/status");
    }

    #[test]
    fn allows_insecure_tls_for_loopback_and_homelab_wsl_hosts() {
        let loopback = reqwest::Url::parse("https://127.0.0.1:4000").expect("loopback URL");
        let homelab = reqwest::Url::parse("https://home-lab-k3s.wsl").expect("homelab URL");
        let nested = reqwest::Url::parse("https://edge.home-lab-k3s.wsl").expect("nested URL");

        assert!(allows_insecure_local_tls(&loopback));
        assert!(allows_insecure_local_tls(&homelab));
        assert!(allows_insecure_local_tls(&nested));
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
}
