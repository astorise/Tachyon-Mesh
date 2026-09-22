use anyhow::{anyhow, bail, Context, Result};
use magnetar_inference_component::{
    ArtifactTrustPolicy, ComponentProviderAdvertisement, InferenceComponentArtifact,
    InferenceComponentPlacement, InferenceComponentSource, LoadedInferenceComponent,
};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::StreamControl;

pub(crate) const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
/// Opaque wire-shape metadata tags (`wit/accelerator/*.wit`'s
/// `invocation-result.metadata`), and one payload paired with its own tags.
type ComponentMetadata = Vec<(String, String)>;
type ComponentInvocationBatch = Vec<(Vec<u8>, ComponentMetadata)>;
/// Trusts the WASM Component binary itself (the code Magnetar instantiates
/// and executes) — never the model weights it happens to load. See
/// [`TACHYON_ARTIFACT_TRUST_STORE_ENV`] for the separate, model-artifact trust
/// decision. Feeds `ArtifactTrustPolicy::trust_component_digest`.
pub(crate) const TACHYON_COMPONENT_TRUST_STORE_ENV: &str = "TACHYON_COMPONENT_TRUST_STORE";
/// Trusts a Model Artifact digest (weights/tokenizer/config bundle) — never
/// the Component binary. See [`TACHYON_COMPONENT_TRUST_STORE_ENV`] for the
/// separate, Component-specific trust decision. Feeds
/// `ArtifactTrustPolicy::trust_digest`.
pub(crate) const TACHYON_ARTIFACT_TRUST_STORE_ENV: &str = "TACHYON_ARTIFACT_TRUST_STORE";

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
        let artifact = component_artifact_from_root(&root)?;
        let trust_policy = tachyon_component_trust_policy(&root)?;
        let component =
            LoadedInferenceComponent::load(alias, artifact, source, trust_policy, placement)
                .with_context(|| {
                    format!(
                        "Magnetar failed to materialize resident inference Component for `{alias}`"
                    )
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

    /// `generate`/`generate_streaming` call `LoadedInferenceComponent`'s
    /// opaque entry points (`invoke_payload_opaque`/
    /// `invoke_payload_streaming_opaque`, astorise/Magnetar#89 — added in
    /// response to the Tachyon integration audit's TACH-02 finding): raw
    /// bytes and string-keyed tags, never Magnetar's own typed generation
    /// output/stream-event shapes. This module touches no generation
    /// vocabulary at all — not a token count, not a stream-event variant —
    /// and neither does anything past it: [`tachyon_wire_tags`] renames
    /// Magnetar's tag keys to Tachyon's own established wire keys by string
    /// lookup, never by typed field access.
    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<ComponentInvocationBatch> {
        if prompts.len() != 1 {
            bail!(
                "Magnetar inference Component invocation for `{}` expects exactly one payload, got {}",
                self.alias,
                prompts.len()
            );
        }
        let outcome = self.component.invoke_payload_opaque(prompts[0])?;
        Ok(vec![(outcome.bytes, tachyon_wire_tags(outcome.tags))])
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_frame: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ComponentMetadata> {
        if prompts.len() != 1 {
            bail!(
                "Magnetar inference Component streaming invocation for `{}` expects exactly one payload, got {}",
                self.alias,
                prompts.len()
            );
        }
        let mut on_event = |frame: &[u8]| -> std::ops::ControlFlow<()> {
            if on_frame(frame).is_stop() {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let tags = self
            .component
            .invoke_payload_streaming_opaque(prompts[0], &mut on_event)?;
        Ok(tachyon_wire_tags(tags))
    }
}

/// Renames Magnetar's own opaque tag keys (`prompt_tokens`,
/// `generated_tokens` — `astorise/Magnetar#89`; it also reports
/// `finish_reason`, not relayed here since Tachyon's wire contract has no
/// slot for it yet) to Tachyon's established wire keys
/// (`wit/accelerator/*.wit`'s `invocation-result.metadata`). Pure
/// string-key lookup — Magnetar's tag keys are read by name, exactly like
/// any other opaque metadata this module never interprets, never through a
/// typed struct field. A key `MagnetarRuntime::generate`/
/// `generate_streaming` don't recognize is silently dropped rather than
/// relayed verbatim, so Tachyon's wire contract stays exactly what it was
/// before Magnetar's opaque entry points existed.
fn tachyon_wire_tags(tags: Vec<(String, String)>) -> ComponentMetadata {
    let by_key: HashMap<String, String> = tags.into_iter().collect();
    let mut wire_tags = Vec::new();
    if let Some(value) = by_key.get("prompt_tokens") {
        wire_tags.push(("tachyon.usage.prompt_tokens".to_owned(), value.clone()));
    }
    if let Some(value) = by_key.get("generated_tokens") {
        wire_tags.push(("tachyon.usage.completion_tokens".to_owned(), value.clone()));
    }
    wire_tags
}

pub(crate) fn is_invalid_component_invocation(error: &anyhow::Error) -> bool {
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

fn component_artifact_from_root(root: &Path) -> Result<InferenceComponentArtifact> {
    let component_paths = std::fs::read_dir(root)
        .with_context(|| {
            format!(
                "failed to read Magnetar Component artifact root `{}`",
                root.display()
            )
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".component.wasm"))
        })
        .collect::<Vec<_>>();

    let component_path = match component_paths.as_slice() {
        [path] => path,
        [] => bail!(
            "Magnetar binding root `{}` must contain an explicit `*.component.wasm` artifact",
            root.display()
        ),
        _ => bail!(
            "Magnetar binding root `{}` must contain exactly one `*.component.wasm` artifact",
            root.display()
        ),
    };
    let manifest_path = component_path.with_file_name(format!(
        "{}.magnetar-component.yaml",
        component_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!(
                "invalid Component artifact path `{}`",
                component_path.display()
            ))?
    ));
    let component_bytes = std::fs::read(component_path)
        .with_context(|| format!("failed to read `{}`", component_path.display()))?;
    let manifest_bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("failed to read `{}`", manifest_path.display()))?;
    Ok(InferenceComponentArtifact::from_bytes(
        component_bytes,
        manifest_bytes,
    ))
}

fn tachyon_component_trust_policy(root: &Path) -> Result<ArtifactTrustPolicy> {
    let mut trust_policy = ArtifactTrustPolicy::default();
    trust_policy = apply_trust_digests(
        trust_policy,
        root,
        TACHYON_COMPONENT_TRUST_STORE_ENV,
        ArtifactTrustPolicy::trust_component_digest,
    )?;
    trust_policy = apply_trust_digests(
        trust_policy,
        root,
        TACHYON_ARTIFACT_TRUST_STORE_ENV,
        ArtifactTrustPolicy::trust_digest,
    )?;
    Ok(trust_policy)
}

/// Reads `trusted_digests` out of the JSON file named by `env_var`, if set,
/// and applies each one to `trust_policy` via `apply`. `TACHYON_COMPONENT_
/// TRUST_STORE` and `TACHYON_ARTIFACT_TRUST_STORE` both use this same file
/// shape and both call this — they differ only in which of `Artifact
/// TrustPolicy`'s two independent trust decisions (Component binary vs.
/// Model Artifact) their digests feed.
fn apply_trust_digests(
    mut trust_policy: ArtifactTrustPolicy,
    root: &Path,
    env_var: &str,
    apply: impl Fn(ArtifactTrustPolicy, &str) -> ArtifactTrustPolicy,
) -> Result<ArtifactTrustPolicy> {
    let Some(trust_path) = std::env::var_os(env_var).map(PathBuf::from) else {
        return Ok(trust_policy);
    };
    reject_trust_policy_inside_artifact(root, &trust_path)?;
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
    for digest in trusted_digests {
        let digest = digest.as_str().ok_or_else(|| {
            anyhow!(
                "`{}` trusted_digests entries must be strings",
                trust_path.display()
            )
        })?;
        trust_policy = apply(trust_policy, digest);
    }
    Ok(trust_policy)
}

fn reject_trust_policy_inside_artifact(root: &Path, trust_path: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize artifact root `{}`", root.display()))?;
    let canonical_trust_path = trust_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize Tachyon Component trust policy `{}`",
            trust_path.display()
        )
    })?;
    if canonical_trust_path.starts_with(&canonical_root) {
        bail!(
            "Tachyon Component trust policy `{}` must be controlled outside artifact root `{}`",
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
