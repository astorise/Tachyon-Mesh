use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmolVmConfig {
    pub image: PathBuf,
    pub vcpus: u8,
    pub memory_mb: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vsock_cid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_command: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuestOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SmolVm {
    config: SmolVmConfig,
}

impl SmolVm {
    pub async fn boot(config: SmolVmConfig) -> Result<Self> {
        validate_config(&config)?;
        Ok(Self { config })
    }

    pub async fn invoke(&self, payload: &[u8]) -> Result<GuestOutput> {
        let Some(agent_command) = self.config.agent_command.as_ref() else {
            return Ok(GuestOutput {
                status: 0,
                stdout: payload.to_vec(),
                stderr: Vec::new(),
            });
        };

        let mut child = Command::new(agent_command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .with_context(|| {
                format!("failed to start guest agent `{}`", agent_command.display())
            })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(payload)
                .await
                .context("failed to write payload to guest agent")?;
        }

        let output = child
            .wait_with_output()
            .await
            .context("failed to collect guest agent output")?;
        Ok(GuestOutput {
            status: output.status.code().unwrap_or(1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    pub fn config(&self) -> &SmolVmConfig {
        &self.config
    }
}

fn validate_config(config: &SmolVmConfig) -> Result<()> {
    if config.vcpus == 0 {
        return Err(anyhow!("smolvm requires at least one vCPU"));
    }
    if config.memory_mb < 64 {
        return Err(anyhow!("smolvm requires at least 64 MiB RAM"));
    }
    if !config.image.exists() {
        return Err(anyhow!(
            "smolvm image `{}` does not exist",
            config.image.display()
        ));
    }
    Ok(())
}
