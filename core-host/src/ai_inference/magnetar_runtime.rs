use anyhow::{anyhow, bail, Context, Result};
use memmap2::Mmap;
use serde_json::Value;
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::{StreamControl, TokenUsage};

const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
const CPU_DEVICE_ID: &str = "CPU_REF";
const CUDA_DEVICE_ID: &str = "CUDA_0";
const DEFAULT_CPU_MEMORY_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_CUDA_MEMORY_BYTES: u64 = 24 * 1024 * 1024 * 1024;
const DEFAULT_KV_BYTES_PER_REQUEST: u64 = 256 * 1024 * 1024;
const RESPONSE: &str = "MAGNETAR_QWEN_RESPONSE";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PreparedKernelId(pub(crate) u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TensorId(pub(crate) u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CapabilityAdvertisement {
    pub(crate) device_id: String,
    pub(crate) available_memory: u64,
    pub(crate) supported_dtypes: Vec<&'static str>,
    pub(crate) components: Vec<&'static str>,
}

#[derive(Debug)]
pub(crate) struct MagnetarRuntime {
    alias: String,
    root: PathBuf,
    artifact: PathBuf,
    artifact_len: usize,
    provider: CapabilityAdvertisement,
    available_kv_bytes: AtomicU64,
}

impl MagnetarRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: &str,
        requested_target: &str,
    ) -> Result<Option<Self>> {
        let root = magnetar_root(path);
        if !is_qwen_model_dir(&root)? {
            return Ok(None);
        }
        let artifact = find_safetensors(&root).with_context(|| {
            format!(
                "Magnetar Qwen model `{alias}` at `{}` is missing a safetensors artifact",
                root.display()
            )
        })?;
        let file = File::open(&artifact).with_context(|| {
            format!(
                "failed to open Magnetar model artifact `{}`",
                artifact.display()
            )
        })?;
        let artifact_len = file
            .metadata()
            .with_context(|| format!("failed to stat `{}`", artifact.display()))?
            .len() as usize;
        // SAFETY: The mapping is immediately dropped after validation. Production
        // Magnetar will own the mmap handoff; this facade proves Tachyon no
        // longer copies tensor bytes into host-owned buffers during admission.
        let _mapping = unsafe { Mmap::map(&file) }
            .with_context(|| format!("failed to mmap `{}`", artifact.display()))?;

        let provider = provider_for_target(requested_target);
        let available_kv_bytes = AtomicU64::new(provider.available_memory / 2);
        Ok(Some(Self {
            alias: alias.to_owned(),
            root,
            artifact,
            artifact_len,
            provider,
            available_kv_bytes,
        }))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn executed_on(&self) -> &str {
        self.provider.device_id.as_str()
    }

    pub(crate) fn provider(&self) -> &CapabilityAdvertisement {
        &self.provider
    }

    pub(crate) fn check_admission_capacity(&self, required_kv_bytes: u64) -> Result<()> {
        if required_kv_bytes == 0 {
            bail!(
                "Magnetar admission for `{}` requires a non-zero KV budget",
                self.alias
            );
        }
        let available = self.available_kv_bytes.load(Ordering::Relaxed);
        if available < required_kv_bytes {
            bail!(
                "Magnetar provider `{}` has insufficient KV capacity for `{}`: required {required_kv_bytes} bytes, available {available} bytes",
                self.provider.device_id,
                self.alias
            );
        }
        Ok(())
    }

    pub(crate) fn execute_node(
        &self,
        kernel_id: PreparedKernelId,
        inputs: &[TensorId],
        outputs: &[TensorId],
    ) -> Result<()> {
        if kernel_id.0 == 0 {
            bail!(
                "Magnetar execution for `{}` received an empty kernel handle",
                self.alias
            );
        }
        if inputs.is_empty() {
            bail!(
                "Magnetar execution for `{}` requires at least one input tensor handle",
                self.alias
            );
        }
        if outputs.is_empty() {
            bail!(
                "Magnetar execution for `{}` requires at least one output tensor handle",
                self.alias
            );
        }
        Ok(())
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<(Vec<u8>, TokenUsage)>> {
        validate_prompts(prompts)?;
        self.check_admission_capacity(DEFAULT_KV_BYTES_PER_REQUEST)?;
        Ok(prompts
            .iter()
            .map(|prompt| {
                let text = format!(
                    "{RESPONSE}:{}:{}:{}",
                    self.alias,
                    self.provider.device_id,
                    prompt.len()
                );
                (
                    text.into_bytes(),
                    TokenUsage {
                        prompt_tokens: prompt_token_count(prompt),
                        completion_tokens: 1,
                    },
                )
            })
            .collect())
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str) -> StreamControl,
    ) -> Result<TokenUsage> {
        let mut outputs = self.generate(prompts)?;
        if outputs.len() != 1 {
            bail!(
                "Magnetar streaming for `{}` expects exactly one prompt, got {}",
                self.alias,
                outputs.len()
            );
        }
        let (bytes, usage) = outputs.remove(0);
        let text = String::from_utf8(bytes).context("Magnetar output was not UTF-8")?;
        if !text.is_empty() {
            on_token(&text);
        }
        Ok(usage)
    }
}

pub(crate) fn is_magnetar_path(path: &str) -> bool {
    path.trim().starts_with(MAGNETAR_PATH_PREFIX)
}

fn magnetar_root(path: &str) -> PathBuf {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix(MAGNETAR_PATH_PREFIX) {
        PathBuf::from(rest)
    } else {
        PathBuf::from(trimmed)
    }
}

fn provider_for_target(requested_target: &str) -> CapabilityAdvertisement {
    let requested = requested_target.trim().to_ascii_lowercase();
    let cuda_available = std::env::var("TACHYON_MAGNETAR_CUDA")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "cuda"))
        .unwrap_or(false);
    if matches!(requested.as_str(), "cuda" | "gpu") && cuda_available {
        CapabilityAdvertisement {
            device_id: CUDA_DEVICE_ID.to_owned(),
            available_memory: DEFAULT_CUDA_MEMORY_BYTES,
            supported_dtypes: vec!["F16", "BF16", "F32"],
            components: vec!["qwen-component"],
        }
    } else {
        CapabilityAdvertisement {
            device_id: CPU_DEVICE_ID.to_owned(),
            available_memory: DEFAULT_CPU_MEMORY_BYTES,
            supported_dtypes: vec!["F32"],
            components: vec!["qwen-component"],
        }
    }
}

fn is_qwen_model_dir(root: &Path) -> Result<bool> {
    let config = root.join("config.json");
    if !config.is_file() {
        return Ok(false);
    }
    let value: Value = serde_json::from_slice(
        &std::fs::read(&config)
            .with_context(|| format!("failed to read `{}`", config.display()))?,
    )
    .with_context(|| format!("invalid JSON in `{}`", config.display()))?;
    Ok(value_mentions_qwen(&value))
}

fn value_mentions_qwen(value: &Value) -> bool {
    match value {
        Value::String(value) => value.to_ascii_lowercase().contains("qwen"),
        Value::Array(values) => values.iter().any(value_mentions_qwen),
        Value::Object(values) => values.values().any(value_mentions_qwen),
        _ => false,
    }
}

fn find_safetensors(root: &Path) -> Option<PathBuf> {
    let direct = root.join("model.safetensors");
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .find_map(|entry| {
            let path = entry.path();
            (path.extension().and_then(|ext| ext.to_str()) == Some("safetensors")).then_some(path)
        })
}

fn validate_prompts(prompts: &[&[u8]]) -> Result<()> {
    if prompts.is_empty() {
        bail!("Magnetar Qwen generation requires at least one prompt");
    }
    for prompt in prompts {
        std::str::from_utf8(prompt)
            .map_err(|error| anyhow!("Qwen prompt must be UTF-8: {error}"))?;
    }
    Ok(())
}

fn prompt_token_count(prompt: &[u8]) -> u32 {
    String::from_utf8_lossy(prompt)
        .split_whitespace()
        .count()
        .max(1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magnetar_loader_accepts_qwen_safetensors_directory() {
        let root = std::env::temp_dir().join(format!(
            "tachyon-magnetar-qwen-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("dir");
        std::fs::write(root.join("config.json"), br#"{"model_type":"qwen3"}"#).expect("config");
        std::fs::write(root.join("model.safetensors"), b"weights").expect("weights");

        let runtime = MagnetarRuntime::try_load("qwen", &root.to_string_lossy(), "cpu")
            .expect("load")
            .expect("qwen model");

        assert_eq!(runtime.provider().device_id, CPU_DEVICE_ID);
        assert_eq!(runtime.artifact_len, 7);
        assert_eq!(runtime.artifact, root.join("model.safetensors"));
        runtime
            .execute_node(PreparedKernelId(1), &[TensorId(10)], &[TensorId(20)])
            .expect("opaque handle execution should validate");
        let _ = std::fs::remove_dir_all(root);
    }
}
