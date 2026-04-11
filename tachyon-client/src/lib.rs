use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;

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
    let raw_lockfile = read_lockfile().await?;
    parse_engine_status(&raw_lockfile)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_engine_status_summarizes_routes_and_batches() {
        let raw_lockfile = r#"{
          "config_payload": "{\"routes\":[{\"path\":\"/api/a\"},{\"path\":\"/api/b\"}],\"batch_targets\":[{\"name\":\"gc-job\"}]}"
        }"#;

        let status = parse_engine_status(raw_lockfile).expect("status should parse");

        assert_eq!(status, "routes=2 batch_targets=1 status=ready");
    }
}
