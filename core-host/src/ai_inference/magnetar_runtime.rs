use anyhow::{Context, Result, anyhow, bail};
use magnetar_inference_component::{
    ArtifactTrustPolicy, InferenceComponentArtifact, InferenceComponentPlacement,
    InferenceComponentSource, LoadedInferenceComponent,
};
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::StreamControl;

pub(crate) const MAGNETAR_PATH_PREFIX: &str = "magnetar:";
/// Opaque wire-shape metadata tags (`wit/accelerator/*.wit`'s
/// `invocation-result.metadata`), and one payload paired with its own tags.
type ComponentMetadata = Vec<(String, String)>;
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

/// A pure Component-invocation bridge: [`MagnetarRuntime`] carries no
/// Provider/Device identity of its own. Which generic accelerator class
/// (`AcceleratorKind`) a bound Component runs on is the placement Tachyon
/// itself requested at load time (kept by the caller alongside this handle,
/// in `ComponentRuntime::Magnetar`) — never something read back out of
/// Magnetar's own Provider advertisement, so there is nothing here to mirror
/// or interpret (audit round 5, TACH-01).
pub(crate) struct MagnetarRuntime {
    alias: String,
    root: PathBuf,
    component: LoadedInferenceComponent,
}

impl std::fmt::Debug for MagnetarRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MagnetarRuntime")
            .field("alias", &self.alias)
            .field("root", &self.root)
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

        Ok(Some(Self {
            alias: alias.to_owned(),
            root,
            component,
        }))
    }

    #[cfg(test)]
    pub(crate) fn resident_debug(&self) -> Result<(String, usize)> {
        self.component.resident_debug()
    }

    /// `invoke`/`invoke_streaming` call `LoadedInferenceComponent`'s opaque
    /// entry points (`invoke_payload_opaque`/`invoke_payload_streaming_opaque`,
    /// astorise/Magnetar#89 — added in response to the Tachyon integration
    /// audit's TACH-02 finding): raw bytes and string-keyed tags, never
    /// Magnetar's own typed generation output/stream-event shapes. This
    /// module touches no generation vocabulary at all — not a token count,
    /// not a finish reason, not a stream-event variant, not even a naming
    /// convention borrowed from one: the prior generation-flavored method
    /// and plural payload-list parameter names are gone (audit round 5,
    /// TACH-02). Whatever tags Magnetar attaches are handed back exactly as
    /// received, by identity, never read, renamed, or filtered by key —
    /// this bridge does not know what any of them mean.
    pub(crate) fn invoke(&self, payload: &[u8]) -> Result<(Vec<u8>, ComponentMetadata)> {
        let outcome = self.component.invoke_payload_opaque(payload)?;
        Ok((outcome.bytes, outcome.tags))
    }

    pub(crate) fn invoke_streaming(
        &self,
        payload: &[u8],
        on_frame: &mut dyn FnMut(&[u8]) -> StreamControl,
    ) -> Result<ComponentMetadata> {
        let mut on_event = |frame: &[u8]| -> std::ops::ControlFlow<()> {
            if on_frame(frame).is_stop() {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let tags = self
            .component
            .invoke_payload_streaming_opaque(payload, &mut on_event)?;
        Ok(tags)
    }
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
