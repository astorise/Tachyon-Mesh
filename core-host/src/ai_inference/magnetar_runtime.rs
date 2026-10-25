#[cfg(feature = "oci-artifacts")]
mod artifact_resolver;

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

/// Name a resolved OCI Component's bytes are always materialized under
/// inside a binding's local root — fixed rather than derived from the
/// locator (which may name an arbitrary repository/tag), so the operator
/// authors the trust manifest sidecar once, under this exact name
/// (`oci-resolved.component.wasm.magnetar-component.yaml`), independent of
/// which locator a binding happens to point at today (TACH-10/13: the
/// published OCI artifact carries only the raw Component bytes, not
/// Magnetar's own manifest sidecar — see `artifact_resolver`'s module
/// doc).
const OCI_RESOLVED_COMPONENT_FILE_NAME: &str = "oci-resolved.component.wasm";
/// Overrides the local content-addressed cache root OCI-resolved artifacts
/// stage into (`<root>/sha256/<digest>/artifact`). Unset falls back to a
/// process-temp-dir-adjacent default — this cache holds no secrets, only
/// already-digest-verified public Component/Kernel bytes, so an
/// unconfigured default is safe, merely inconvenient to reuse across
/// restarts.
const TACHYON_OCI_ARTIFACT_CACHE_ENV: &str = "TACHYON_OCI_ARTIFACT_CACHE";

impl MagnetarRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: &str,
        requested_target: &str,
        component_locator: Option<&str>,
    ) -> Result<Option<Self>> {
        if !is_magnetar_path(path) {
            return Ok(None);
        }

        let root = magnetar_root(path);
        if let Some(locator) = component_locator {
            materialize_oci_component(&root, locator)?;
        }
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

/// Resolves `locator` (an `oci://...@sha256:...` reference — see
/// `artifact_resolver::ArtifactReference`) and copies the verified bytes
/// into `root` under [`OCI_RESOLVED_COMPONENT_FILE_NAME`], so the existing,
/// unmodified `component_artifact_from_root` finds them exactly as it
/// would a hand-placed local `*.component.wasm` file — an `oci://`-sourced
/// binding behaves identically, end to end, to a fully local one past this
/// point (TACH-10/13's transport-neutrality requirement).
#[cfg(feature = "oci-artifacts")]
fn materialize_oci_component(root: &Path, locator: &str) -> Result<()> {
    let reference = artifact_resolver::ArtifactReference::parse(locator)?;
    let cache_root = std::env::var_os(TACHYON_OCI_ARTIFACT_CACHE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("tachyon-oci-artifacts"));
    let resolved_path = artifact_resolver::resolve(&reference, &cache_root)?;
    let bytes = std::fs::read(&resolved_path).with_context(|| {
        format!(
            "failed to read resolved OCI artifact at `{}`",
            resolved_path.display()
        )
    })?;
    std::fs::create_dir_all(root).with_context(|| {
        format!(
            "failed to create Component binding root `{}`",
            root.display()
        )
    })?;
    std::fs::write(root.join(OCI_RESOLVED_COMPONENT_FILE_NAME), bytes).with_context(|| {
        format!(
            "failed to materialize resolved OCI Component into `{}`",
            root.display()
        )
    })?;
    Ok(())
}

#[cfg(not(feature = "oci-artifacts"))]
fn materialize_oci_component(_root: &Path, locator: &str) -> Result<()> {
    bail!(
        "Component binding names an OCI locator (`{locator}`) but this build was compiled \
         without the `oci-artifacts` feature"
    )
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

/// TACH-10/TACH-13 (2026-09-24 audit): a real end-to-end proof against the
/// actual Magnetar artifact published to GHCR by
/// `component-artifact-distribution.yml` run #2
/// (<https://github.com/astorise/Magnetar/actions/runs/39452249157>) --
/// never a simulated registry or a fabricated digest. This is the one test
/// in this module that touches the network; it is not behind any extra
/// opt-in flag beyond the `oci-artifacts` feature itself, matching this
/// codebase's existing convention of failing hard rather than silently
/// skipping when a required external resource is unreachable (see
/// `TACHYON_REQUIRE_GUEST_OPENAI`).
#[cfg(all(test, feature = "oci-artifacts"))]
mod oci_artifact_tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    /// The real, checked-in-on-Magnetar-main manifest sidecar for
    /// `synthetic-minimal.component.wasm`, carrying that Component's real
    /// digest (`sha256:467bfdf6...`) -- the exact bytes GHCR serves for
    /// `ghcr.io/astorise/magnetar-component:synthetic-minimal-latest`, per
    /// that push's own job log. `signatures: []` here is the real fixture's
    /// own content (TACH-11 is proven separately, with local test keys —
    /// this artifact was never signed).
    const SYNTHETIC_MINIMAL_MANIFEST_YAML: &str = r#"schema: magnetar-component-artifact
schema_version: 1
artifact:
  kind: component
  digest:
    algorithm: sha256
    value: "sha256:467bfdf663fcdb6034294599f6a079a9e0af05c1d0be2e8996afe1c2ef463854"
component:
  name: "magnetar.test.synthetic-minimal"
  version: "0.1.0"
  description: "Test-only Model Component fixture, published as the real GHCR OCI artifact this test resolves."
  role: "qwen-model-component"
runtime:
  magnetar:
    min_version: "0.1.0"
wit:
  imports:
    - package: "magnetar:model-component-graph"
      interface: "graph-builder"
      version: "1.2.0"
    - package: "magnetar:model-component-graph"
      interface: "model-config"
      version: "1.2.0"
  exports: []
capabilities:
  requires: []
authority:
  requires: []
engine:
  profile: "native"
  features:
    - component-model
    - resource-limits
publisher:
  id: "local-dev"
  name: "Local Development"
source:
  kind: "local"
  uri: "./oci-resolved.component.wasm"
signatures: []
"#;

    const REAL_COMPONENT_LOCATOR: &str = "oci://ghcr.io/astorise/magnetar-component:synthetic-minimal-latest@sha256:bb393658899d36d36dda37224861c4437f249b6e240d196585a2eea17c3fe214";
    const REAL_COMPONENT_DIGEST: &str =
        "sha256:467bfdf663fcdb6034294599f6a079a9e0af05c1d0be2e8996afe1c2ef463854";

    /// Serializes this module's tests against each other: both mutate the
    /// process-wide `TACHYON_COMPONENT_TRUST_STORE`/`TACHYON_OCI_ARTIFACT_
    /// CACHE` environment variables, which `cargo test`'s default parallel
    /// execution would otherwise race.
    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous_cache: Option<std::ffi::OsString>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // FIXME: Audit that the environment access only happens in single-threaded code.
            unsafe {
                match self.previous_cache.take() {
                    Some(previous) => std::env::set_var(TACHYON_OCI_ARTIFACT_CACHE_ENV, previous),
                    None => std::env::remove_var(TACHYON_OCI_ARTIFACT_CACHE_ENV),
                }
            }
        }
    }

    /// Covers `materialize_oci_component` + `component_artifact_from_root`
    /// (the exact pair `try_load` runs in sequence) against the real,
    /// network-pulled bytes -- proving an `oci://`-sourced binding behaves
    /// identically to a hand-placed local one at every layer transport
    /// cannot affect. Stops short of `LoadedInferenceComponent::load`
    /// itself: that call additionally ingests a Model Artifact bundle from
    /// the same root regardless of which Component is loaded, a
    /// requirement orthogonal to transport that `synthetic-minimal` (a
    /// Component-registration fixture, never paired with a Model Artifact
    /// bundle anywhere in Magnetar's own test suite either) was never
    /// meant to satisfy.
    #[test]
    fn resolves_and_registers_the_real_published_ghcr_component_end_to_end() {
        let lock = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("temp dir creates");
        let cache_dir = temp.path().join("cache");
        let binding_root = temp.path().join("binding");
        std::fs::create_dir_all(&binding_root).expect("binding root creates");

        std::fs::write(
            binding_root.join(format!(
                "{OCI_RESOLVED_COMPONENT_FILE_NAME}.magnetar-component.yaml"
            )),
            SYNTHETIC_MINIMAL_MANIFEST_YAML,
        )
        .expect("manifest sidecar writes");

        let previous_cache = std::env::var_os(TACHYON_OCI_ARTIFACT_CACHE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(TACHYON_OCI_ARTIFACT_CACHE_ENV, &cache_dir) };
        let _guard = EnvGuard {
            _lock: lock,
            previous_cache,
        };

        // Real network pull against the real artifact GHCR actually
        // serves, verified by manifest digest, staged into the local CAS,
        // then materialized into the binding root under the fixed name the
        // manifest sidecar above names.
        materialize_oci_component(&binding_root, REAL_COMPONENT_LOCATOR)
            .expect("a real, currently-published GHCR locator must resolve and materialize");

        let materialized = binding_root.join(OCI_RESOLVED_COMPONENT_FILE_NAME);
        let materialized_bytes = std::fs::read(&materialized).expect("materialized file reads");
        assert_eq!(
            format!("sha256:{}", sha256_hex_for_test(&materialized_bytes)),
            REAL_COMPONENT_DIGEST,
            "the materialized bytes must be exactly the real, digest-verified GHCR artifact"
        );

        // Same call `try_load` makes next: the existing, unmodified local-
        // directory scan finds the materialized bytes and their sidecar
        // manifest exactly as it would a hand-placed local bundle --
        // `component_artifact_from_root`'s behavior from here on is
        // determined entirely by the bytes on disk (already verified
        // above by digest), with no record of which transport put them
        // there. Its return value is opaque by design (`InferenceComponent
        // Artifact`'s fields are private to `inference-components`, by
        // construction only consumable via `LoadedInferenceComponent::
        // load`, which additionally requires a paired Model Artifact
        // bundle this Component-registration-only fixture was never meant
        // to have -- see this test's own doc comment), so succeeding
        // without error is what this call proves: the OCI-materialized
        // files are discoverable exactly like a hand-placed local bundle.
        component_artifact_from_root(&binding_root).expect(
            "the materialized bytes and sidecar manifest must be discoverable exactly like a \
             hand-placed local bundle",
        );

        // Second resolve: same locator, now served entirely from the local
        // CAS cache populated above -- proving the cache-hit path, not
        // just the resolver in isolation.
        let cache_artifact_path = cache_dir
            .join("sha256")
            .join(
                REAL_COMPONENT_LOCATOR
                    .rsplit("sha256:")
                    .next()
                    .expect("rsplit always yields at least one segment"),
            )
            .join("artifact");
        assert!(
            cache_artifact_path.is_file(),
            "the first resolve must have staged the artifact into the local CAS"
        );
        let staged_before = std::fs::metadata(&cache_artifact_path)
            .expect("staged artifact metadata reads")
            .modified()
            .expect("mtime is available on this platform");

        materialize_oci_component(&binding_root, REAL_COMPONENT_LOCATOR)
            .expect("a cache-hit resolve must still succeed");
        let staged_after = std::fs::metadata(&cache_artifact_path)
            .expect("staged artifact metadata reads")
            .modified()
            .expect("mtime is available on this platform");
        assert_eq!(
            staged_before, staged_after,
            "a cache hit must not re-stage (and therefore must not re-fetch) the artifact"
        );
    }

    /// The same real, currently-published tag as `REAL_COMPONENT_LOCATOR`,
    /// but paired with a digest that is not the one GHCR actually serves
    /// for it -- proving the resolver's manifest-digest check is a real
    /// rejection, not merely a pass-through of whatever the registry
    /// returns under that tag.
    #[test]
    fn rejects_a_real_tag_resolving_to_an_unexpected_digest() {
        let lock = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("temp dir creates");
        let cache_dir = temp.path().join("cache");
        let binding_root = temp.path().join("binding");
        std::fs::create_dir_all(&binding_root).expect("binding root creates");

        let previous_cache = std::env::var_os(TACHYON_OCI_ARTIFACT_CACHE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(TACHYON_OCI_ARTIFACT_CACHE_ENV, &cache_dir) };
        let _guard = EnvGuard {
            _lock: lock,
            previous_cache,
        };

        let wrong_digest_locator = "oci://ghcr.io/astorise/magnetar-component:synthetic-minimal-latest@sha256:0000000000000000000000000000000000000000000000000000000000000000";
        let error = materialize_oci_component(&binding_root, wrong_digest_locator)
            .expect_err("a tag that resolves to a different real digest must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("digest"),
            "the rejection must name the digest mismatch, got: {message}"
        );
    }

    /// Regression test for a real finding: `resolve`/`materialize_oci_
    /// component` are plain synchronous functions reachable from `core-
    /// host`'s fully synchronous AI-inference load path, which `main`'s
    /// `#[tokio::main]` entrypoint calls directly (no `spawn_blocking`, no
    /// `.await`) while building the route registry at host startup.
    /// `reqwest::blocking::Client` builds and drives its own Tokio runtime
    /// internally, and entering it from a thread that is already a Tokio
    /// runtime worker panics ("Cannot start a runtime from within a
    /// runtime") -- so, before `pull_off_any_tokio_runtime_thread` existed,
    /// a cache-miss resolve reached from exactly this call shape crashed a
    /// fresh host outright instead of returning a typed load error. This
    /// test reproduces that call shape directly: a plain, non-`.await`ed
    /// call to `materialize_oci_component` from inside a running Tokio
    /// runtime's `block_on`, against a real cache miss that forces a real
    /// network pull -- the exact path that used to panic.
    #[test]
    fn resolving_from_inside_an_active_tokio_runtime_does_not_panic() {
        let lock = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = tempfile::tempdir().expect("temp dir creates");
        let cache_dir = temp.path().join("cache");
        let binding_root = temp.path().join("binding");
        std::fs::create_dir_all(&binding_root).expect("binding root creates");

        let previous_cache = std::env::var_os(TACHYON_OCI_ARTIFACT_CACHE_ENV);
        // FIXME: Audit that the environment access only happens in single-threaded code.
        unsafe { std::env::set_var(TACHYON_OCI_ARTIFACT_CACHE_ENV, &cache_dir) };
        let _guard = EnvGuard {
            _lock: lock,
            previous_cache,
        };

        let runtime = tokio::runtime::Runtime::new().expect("a Tokio runtime must build");
        let result = runtime.block_on(async {
            // No `.await`, no `spawn_blocking` -- this is the exact
            // synchronous call shape `AiInferenceRuntime::from_config`
            // uses from `build_runtime_state`, called directly from an
            // `async fn` while already inside this runtime's `block_on`.
            materialize_oci_component(&binding_root, REAL_COMPONENT_LOCATOR)
        });
        result.expect(
            "resolving a real OCI locator from inside an active Tokio runtime must succeed, \
             not panic, on a cache miss that forces a real network pull",
        );
    }

    fn sha256_hex_for_test(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
