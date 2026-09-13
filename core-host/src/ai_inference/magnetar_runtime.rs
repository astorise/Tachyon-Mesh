use anyhow::{anyhow, bail, Context, Result};
use magnetar_inference_component::{
    ComponentProviderAdvertisement, InferenceComponentPlacement, InferenceComponentSource,
    LoadedInferenceComponent,
};
use magnetar_runtime::model::ModelTrustStore;
use magnetar_runtime::GenerationStreamEvent;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::{StreamControl, TokenUsage};

pub(crate) const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
pub(crate) const TACHYON_MODEL_TRUST_STORE_ENV: &str = "TACHYON_MODEL_TRUST_STORE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderAdvertisement {
    pub(crate) provider_name: String,
    pub(crate) provider_version: String,
    pub(crate) device_ids: Vec<String>,
    pub(crate) device_class: ProviderDeviceClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderDeviceClass {
    ReferenceCpu,
    Cuda,
}

pub(crate) struct MagnetarRuntime {
    alias: String,
    root: PathBuf,
    provider: ProviderAdvertisement,
    component: LoadedInferenceComponent,
}

impl std::fmt::Debug for MagnetarRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MagnetarRuntime")
            .field("alias", &self.alias)
            .field("root", &self.root)
            .field("provider", &self.provider)
            .field("target", &self.provider.device_class)
            .finish_non_exhaustive()
    }
}

impl MagnetarRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: &str,
        requested_target: &str,
    ) -> Result<Option<Self>> {
        if !is_magnetar_path(path) {
            return Ok(None);
        }

        let root = magnetar_root(path);
        let placement = InferenceComponentPlacement::parse(requested_target)?;
        let source = InferenceComponentSource::authorized_local_bundle(
            format!("tachyon:{alias}"),
            root.clone(),
        );
        let trust_store = tachyon_model_trust_store(&root)?;
        let component = LoadedInferenceComponent::load(alias, source, trust_store, placement)
            .with_context(|| {
                format!("Magnetar failed to materialize resident inference Component for `{alias}`")
            })?;
        let provider = provider_advertisement(component.provider());

        Ok(Some(Self {
            alias: alias.to_owned(),
            root,
            provider,
            component,
        }))
    }

    pub(crate) fn executed_on(&self) -> &str {
        self.provider.provider_name.as_str()
    }

    pub(crate) fn provider(&self) -> &ProviderAdvertisement {
        &self.provider
    }

    #[cfg(test)]
    pub(crate) fn resident_debug(&self) -> Result<(String, usize)> {
        self.component.resident_debug()
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<(Vec<u8>, TokenUsage)>> {
        if prompts.len() != 1 {
            bail!(
                "Magnetar inference Component invocation for `{}` expects exactly one payload, got {}",
                self.alias,
                prompts.len()
            );
        }
        let outcome = self.component.invoke_payload(prompts[0])?;
        let usage = outcome.usage;
        Ok(vec![(
            outcome.text.into_bytes(),
            TokenUsage {
                prompt_tokens: usage.prompt_tokens.min(u32::MAX as usize) as u32,
                completion_tokens: usage.generated_tokens.min(u32::MAX as usize) as u32,
            },
        )])
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str) -> StreamControl,
    ) -> Result<TokenUsage> {
        if prompts.len() != 1 {
            bail!(
                "Magnetar inference Component streaming invocation for `{}` expects exactly one payload, got {}",
                self.alias,
                prompts.len()
            );
        }
        let mut streamed_usage = None;
        let mut on_event = |event: GenerationStreamEvent| -> std::ops::ControlFlow<()> {
            match event {
                GenerationStreamEvent::Token {
                    text_delta: Some(delta),
                    ..
                } if !delta.is_empty() => {
                    if on_token(&delta).is_stop() {
                        std::ops::ControlFlow::Break(())
                    } else {
                        std::ops::ControlFlow::Continue(())
                    }
                }
                GenerationStreamEvent::Finished { usage, .. } => {
                    streamed_usage = Some(TokenUsage {
                        prompt_tokens: usage.prompt_tokens.min(u32::MAX as usize) as u32,
                        completion_tokens: usage.generated_tokens.min(u32::MAX as usize) as u32,
                    });
                    std::ops::ControlFlow::Continue(())
                }
                _ => std::ops::ControlFlow::Continue(()),
            }
        };
        let outcome = self
            .component
            .invoke_payload_streaming(prompts[0], &mut on_event)?;
        Ok(streamed_usage.unwrap_or_else(|| TokenUsage {
            prompt_tokens: outcome.prompt_tokens.min(u32::MAX as usize) as u32,
            completion_tokens: outcome.generated_tokens.min(u32::MAX as usize) as u32,
        }))
    }
}

pub(crate) fn is_invalid_generation_request(error: &anyhow::Error) -> bool {
    magnetar_inference_component::is_invalid_component_invocation(error)
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

fn tachyon_model_trust_store(root: &Path) -> Result<ModelTrustStore> {
    let Some(trust_path) = std::env::var_os(TACHYON_MODEL_TRUST_STORE_ENV).map(PathBuf::from)
    else {
        return Ok(ModelTrustStore::default());
    };
    reject_trust_store_inside_artifact(root, &trust_path)?;
    let value = serde_json::from_slice::<Value>(
        &std::fs::read(&trust_path)
            .with_context(|| format!("failed to read `{}`", trust_path.display()))?,
    )
    .with_context(|| format!("failed to parse `{}`", trust_path.display()))?;
    let trusted_digests = value
        .get("trusted_digests")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow!(
                "`{}` must contain a `trusted_digests` array",
                trust_path.display()
            )
        })?;
    let mut trust_store = ModelTrustStore::default();
    for digest in trusted_digests {
        let digest = digest.as_str().ok_or_else(|| {
            anyhow!(
                "`{}` trusted_digests entries must be strings",
                trust_path.display()
            )
        })?;
        trust_store = trust_store.trust_digest(digest);
    }
    Ok(trust_store)
}

fn reject_trust_store_inside_artifact(root: &Path, trust_path: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize model root `{}`", root.display()))?;
    let canonical_trust_path = trust_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize Tachyon trust store `{}`",
            trust_path.display()
        )
    })?;
    if canonical_trust_path.starts_with(&canonical_root) {
        bail!(
            "Tachyon model trust store `{}` must be controlled outside artifact root `{}`",
            canonical_trust_path.display(),
            canonical_root.display()
        );
    }
    Ok(())
}

fn provider_advertisement(provider: &ComponentProviderAdvertisement) -> ProviderAdvertisement {
    ProviderAdvertisement {
        provider_name: provider.provider_name.clone(),
        provider_version: provider.provider_version.clone(),
        device_ids: provider.device_ids.clone(),
        device_class: match provider.device_class {
            magnetar_inference_component::ProviderDeviceClass::ReferenceCpu => {
                ProviderDeviceClass::ReferenceCpu
            }
            magnetar_inference_component::ProviderDeviceClass::Cuda => ProviderDeviceClass::Cuda,
        },
    }
}
