use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf, time::Duration};

const AGENT_COMMAND_ENV: &str = "TACHYON_SMOLVM_AGENT_COMMAND";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MicroVmConfig {
    pub image: PathBuf,
    pub vcpus: u8,
    pub memory_mb: u32,
    #[serde(default)]
    pub keep_warm: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vsock_cid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_path: Option<PathBuf>,
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

#[derive(Debug)]
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
        let output =
            tokio::time::timeout(Duration::from_secs(30), self.invoke_guest_agent(payload))
                .await
                .context("microvm invocation timed out")??;
        Ok(output)
    }

    async fn invoke_guest_agent(&self, payload: Vec<u8>) -> Result<MicroVmResult> {
        let vm = smolvm::SmolVm::boot(smolvm::SmolVmConfig {
            image: self.config.image.clone(),
            vcpus: self.config.vcpus,
            memory_mb: self.config.memory_mb,
            tap_device: self.config.tap_device.clone(),
            vsock_cid: self.config.vsock_cid,
            serial_path: self.config.serial_path.clone(),
            snapshot_path: self.config.snapshot_path.clone(),
            agent_command: env::var_os(AGENT_COMMAND_ENV).map(PathBuf::from),
        })
        .await
        .context("failed to boot smolvm guest")?;
        let output = vm
            .invoke(&payload)
            .await
            .context("failed to exchange payload with smolvm guest agent")?;
        Ok(MicroVmResult {
            status: output.status,
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
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
            tap_device: None,
            vsock_cid: None,
            serial_path: None,
            snapshot_path: None,
        })
        .expect_err("zero vcpu should be rejected");
        assert!(err.to_string().contains("vCPU"));
    }

    #[tokio::test]
    async fn invokes_guest_agent_bridge() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-smolvm-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let image = dir.join("numpy.smolmachine");
        std::fs::write(&image, b"mock image").expect("image should be written");

        let runner = MicroVmRunner::new(MicroVmConfig {
            image,
            vcpus: 1,
            memory_mb: 256,
            keep_warm: false,
            tap_device: Some("tap-tachyon-test".to_owned()),
            vsock_cid: Some(42),
            serial_path: Some(dir.join("serial.sock")),
            snapshot_path: Some(dir.join("snapshot.bin")),
        })
        .expect("runner config should be valid");

        let result = runner
            .invoke(MicroVmInvocation {
                module_id: "numpy-matrix".to_owned(),
                payload: serde_json::json!({
                    "script": "import numpy as np; print(np.matmul([[1,2]], [[3],[4]]))",
                    "expected": 11
                }),
            })
            .await
            .expect("mock smolvm invocation should succeed");

        assert_eq!(result.status, 0);
        assert!(result.stdout.contains("numpy-matrix"));
    }
}
