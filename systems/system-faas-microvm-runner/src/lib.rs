use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroVmConfig {
    pub image: PathBuf,
    pub vcpus: u8,
    pub memory_mb: u32,
    #[serde(default)]
    pub keep_warm: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroVmInvocation {
    pub module_id: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroVmResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct MicroVmRunner {
    config: MicroVmConfig,
}

impl MicroVmRunner {
    pub fn new(config: MicroVmConfig) -> Result<Self> {
        if config.vcpus == 0 {
            return Err(anyhow!("microvm runtime requires at least one vCPU"));
        }
        if config.memory_mb < 64 {
            return Err(anyhow!("microvm runtime requires at least 64 MiB RAM"));
        }
        if !config.image.exists() {
            return Err(anyhow!(
                "microvm image `{}` does not exist",
                config.image.display()
            ));
        }
        Ok(Self { config })
    }

    pub async fn invoke(&self, invocation: MicroVmInvocation) -> Result<MicroVmResult> {
        let payload =
            serde_json::to_vec(&invocation).context("failed to encode microvm invocation")?;
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            self.invoke_guest_agent(payload),
        )
        .await
        .context("microvm invocation timed out")??;
        Ok(output)
    }

    async fn invoke_guest_agent(&self, payload: Vec<u8>) -> Result<MicroVmResult> {
        let image = self.config.image.display().to_string();
        Ok(MicroVmResult {
            status: 501,
            stdout: String::new(),
            stderr: format!(
                "SmolVM SDK bridge is not linked in this build; image={image}; payloadBytes={}",
                payload.len()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_vcpu_config() {
        let err = MicroVmRunner::new(MicroVmConfig {
            image: PathBuf::from("missing.smolmachine"),
            vcpus: 0,
            memory_mb: 256,
            keep_warm: false,
        })
        .expect_err("zero vcpu should be rejected");
        assert!(err.to_string().contains("vCPU"));
    }
}
