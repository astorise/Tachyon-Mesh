//! TACH-10/TACH-13 (2026-09-24 audit, `consume-magnetar-oci-artifacts`):
//! resolves an `oci://` locator into a local, digest-verified file, staged
//! in a content-addressed cache. Deliberately minimal rather than a
//! dependency on a general-purpose OCI client crate (gated behind the
//! `oci-artifacts` feature either way, so a build that never uses it pays
//! nothing extra): this only ever needs to GET a single-manifest, single-
//! layer artifact by tag and verify two digests, using HTTP machinery
//! (`reqwest`, already a mandatory core-host dependency for the upstream
//! OpenAI-scheme remote provider path) and hashing (`sha2`, likewise
//! already mandatory) this crate already carries for unrelated reasons.
//!
//! What this module is NOT: a general OCI client, a registry push path, or
//! a place that knows anything about Magnetar's own Component/Kernel
//! manifest shape. It hands back verified bytes at a local path; Magnetar
//! trust/ingestion starts only after that, exactly as it does for a
//! hand-placed local bundle (`MagnetarRuntime` itself gains no new surface
//! -- see `consume-magnetar-oci-artifacts/proposal.md`'s "what the bridge
//! must not become").
//!
//! # Trust model
//!
//! `ArtifactReference::expected_digest` is the only trust identity. The
//! locator's tag is a transport hint only: this resolver always computes
//! the actual digest of what it pulled and rejects a mismatch outright,
//! whether that is a tag repointed to a different manifest or a corrupted
//! local cache entry. Never trusts a registry-supplied `Docker-Content-
//! Digest` response header on its own -- that header is checked against
//! the independently *computed* digest, never substituted for it.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// `oci://<registry>/<repository>:<tag>@sha256:<64-hex-digest>` -- tag and
/// digest both required, deliberately: the tag names *what to fetch*, the
/// digest names *what is trusted*, and conflating them (e.g. trusting
/// whatever a tag currently resolves to) is exactly the failure mode this
/// module exists to close.
pub(crate) const OCI_LOCATOR_PREFIX: &str = "oci://";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactReference {
    pub(crate) registry: String,
    pub(crate) repository: String,
    pub(crate) tag: String,
    pub(crate) expected_digest: String,
}

impl ArtifactReference {
    pub(crate) fn parse(locator: &str) -> Result<Self> {
        let rest = locator.strip_prefix(OCI_LOCATOR_PREFIX).ok_or_else(|| {
            anyhow!("OCI artifact locator `{locator}` must start with `{OCI_LOCATOR_PREFIX}`")
        })?;
        let (reference, expected_digest) = rest
            .split_once('@')
            .ok_or_else(|| anyhow!("OCI artifact locator `{locator}` must carry an explicit `@sha256:<digest>` -- a tag alone is never a trust identity"))?;
        if !is_canonical_sha256(expected_digest) {
            bail!(
                "OCI artifact locator `{locator}` digest must be canonical `sha256:<64 lowercase hex>`, got `{expected_digest}`"
            );
        }
        let (registry_and_repo, tag) = reference.rsplit_once(':').ok_or_else(|| {
            anyhow!(
                "OCI artifact locator `{locator}` must name an explicit tag (`<repository>:<tag>`)"
            )
        })?;
        let (registry, repository) = registry_and_repo
            .split_once('/')
            .ok_or_else(|| anyhow!("OCI artifact locator `{locator}` must name a registry host (`<registry>/<repository>`)"))?;
        if registry.is_empty() || repository.is_empty() || tag.is_empty() {
            bail!("OCI artifact locator `{locator}` has an empty registry, repository, or tag");
        }
        Ok(Self {
            registry: registry.to_owned(),
            repository: repository.to_owned(),
            tag: tag.to_owned(),
            expected_digest: expected_digest.to_ascii_lowercase(),
        })
    }
}

fn is_canonical_sha256(digest: &str) -> bool {
    digest
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Deserialize)]
struct OciManifest {
    layers: Vec<OciManifestLayer>,
}

#[derive(Deserialize)]
struct OciManifestLayer {
    digest: String,
}

#[derive(Deserialize)]
struct RegistryTokenResponse {
    token: String,
}

/// Resolves `reference` into a local file path, using `cache_root` as a
/// content-addressed cache keyed by `expected_digest` -- the *manifest's*
/// digest (`<cache_root>/sha256/<manifest-digest>/`), matching exactly
/// what `oras resolve`/a registry UI reports as "the digest" of a tag, and
/// therefore what an operator actually has in hand to pin. The manifest
/// digest is verified first, against the raw manifest bytes, before its
/// *declared* layer digest is trusted for anything -- a registry serving
/// a self-consistent but wrong manifest is caught here, not just a
/// corrupted blob under an otherwise-correct one. The cache entry holds
/// the raw `manifest.json` bytes alongside the layer's own content
/// (`artifact`); the layer digest used for every later corruption check is
/// derived by re-parsing `manifest.json` itself, not a separate flat
/// sidecar, so a cache hit re-verifies the *whole* chain
/// (`expected_digest` -> manifest -> declared layer digest -> artifact
/// bytes) rather than trusting a number that was never itself re-tied to
/// `expected_digest` (TACH-15: a coherent replacement of just the old
/// `artifact`/`layer-digest.txt` pair could previously pass a cache hit
/// without those bytes ever having been bound to this locator).
///
/// Cache-hit path (the common case once an artifact has been pulled once)
/// takes no network access at all (`Requirement: Corrupted Cache Entries
/// Are Detected Before Load`). A missing cache entry -- or one with a
/// manifest present but the artifact itself missing, e.g. an interrupted
/// stage -- is treated as a cache miss (safe: it only costs a re-fetch),
/// never as a reason to trust an unverified file. A manifest.json or
/// artifact that *is* present but no longer ties back to `expected_digest`
/// is a different thing entirely -- rejected outright, not silently
/// re-fetched, since a soft retry there could paper over tampering
/// instead of surfacing it.
pub(crate) fn resolve(reference: &ArtifactReference, cache_root: &Path) -> Result<PathBuf> {
    let cache_dir = cache_root
        .join("sha256")
        .join(digest_hex(&reference.expected_digest));
    let manifest_path = cache_dir.join("manifest.json");
    let artifact_path = cache_dir.join("artifact");

    if let Some(resolved) = revalidate_cache_entry(reference, &manifest_path, &artifact_path)? {
        return Ok(resolved);
    }

    let pulled = pull_off_any_tokio_runtime_thread(reference)?;
    stage_cache_entry(&cache_dir, &pulled.manifest_bytes, &pulled.layer_bytes)?;

    revalidate_cache_entry(reference, &manifest_path, &artifact_path)?.ok_or_else(|| {
        anyhow!(
            "internal error: OCI artifact cache entry at `{}` failed its own revalidation immediately after being staged",
            cache_dir.display()
        )
    })
}

/// `Ok(None)`: no cache entry present (or only a partially-staged one),
/// safe to pull. `Ok(Some(path))`: a cache entry is present and its
/// content cryptographically re-ties, right now, to `reference`. `Err`:
/// a cache entry is present but does not -- a fail-closed rejection, not a
/// miss, exactly like a first pull's own manifest/layer digest checks.
fn revalidate_cache_entry(
    reference: &ArtifactReference,
    manifest_path: &Path,
    artifact_path: &Path,
) -> Result<Option<PathBuf>> {
    let Ok(manifest_bytes) = fs::read(manifest_path) else {
        return Ok(None);
    };
    let actual_manifest_digest = format!("sha256:{}", sha256_hex(&manifest_bytes));
    if actual_manifest_digest != reference.expected_digest {
        bail!(
            "cached OCI manifest at `{}` no longer matches the digest its cache key promises \
             (expected `{}`, computed `{actual_manifest_digest}`) -- the cache entry is corrupted \
             and is rejected rather than handed to Magnetar",
            manifest_path.display(),
            reference.expected_digest,
        );
    }

    let manifest: OciManifest = serde_json::from_slice(&manifest_bytes).with_context(|| {
        format!(
            "cached OCI manifest at `{}` carries the digest its cache key promises but is not valid JSON",
            manifest_path.display()
        )
    })?;
    let layer = single_layer(&manifest, &reference.repository, &reference.tag)?;
    let declared_layer_digest = layer.digest.to_ascii_lowercase();

    let Ok(artifact_bytes) = fs::read(artifact_path) else {
        return Ok(None);
    };
    let actual_layer_digest = format!("sha256:{}", sha256_hex(&artifact_bytes));
    if actual_layer_digest != declared_layer_digest {
        bail!(
            "cached artifact at `{}` no longer matches its own cached manifest's declared layer digest \
             (declared `{declared_layer_digest}`, computed `{actual_layer_digest}`) -- the cache entry \
             is corrupted and is rejected rather than handed to Magnetar",
            artifact_path.display(),
        );
    }

    Ok(Some(artifact_path.to_path_buf()))
}

fn single_layer<'a>(
    manifest: &'a OciManifest,
    repository: &str,
    tag: &str,
) -> Result<&'a OciManifestLayer> {
    match manifest.layers.as_slice() {
        [layer] => Ok(layer),
        [] => bail!("OCI manifest for `{repository}:{tag}` carries no layers"),
        _ => bail!(
            "OCI manifest for `{repository}:{tag}` carries {} layers; this resolver only supports a single-layer artifact",
            manifest.layers.len()
        ),
    }
}

struct PulledArtifact {
    manifest_bytes: Vec<u8>,
    layer_bytes: Vec<u8>,
}

/// `reqwest::blocking::Client` builds and drives its own Tokio runtime
/// internally; entering it from a thread that is itself already a Tokio
/// runtime worker panics ("Cannot start a runtime from within a runtime").
/// `resolve` is a plain synchronous fn reachable from `core-host`'s fully
/// synchronous AI-inference load path, which `main`'s `#[tokio::main]`
/// entrypoint calls directly (no `spawn_blocking`, no `.await`) while
/// building the route registry at host startup -- so a cache-miss pull
/// here would otherwise crash a fresh host outright instead of returning
/// a typed load error. Running `pull` on a plain OS thread, never a Tokio
/// one, sidesteps that regardless of which thread `resolve` itself runs
/// on; the calling thread's `.join()` is an ordinary blocking join, not a
/// runtime method, so it does not trip the same guard.
fn pull_off_any_tokio_runtime_thread(reference: &ArtifactReference) -> Result<PulledArtifact> {
    let reference = reference.clone();
    std::thread::spawn(move || pull(&reference))
        .join()
        .map_err(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .map(|value| value.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            anyhow!("OCI artifact pull thread panicked: {message}")
        })?
}

/// Removes every cache entry under `cache_root/sha256/<digest>` whose
/// digest is not in `referenced_digests` (`Cache lifecycle: garbage
/// collection for unreferenced digests`). `referenced_digests` is the full
/// set of `expected_digest` values (`sha256:<64 hex>`) that at least one
/// currently-configured binding still names; a cache entry for any other
/// digest can no longer be reached by any live binding, so it is safe to
/// reclaim. Returns the number of entries removed. A `cache_root` that has
/// never had anything staged into it (no `sha256/` directory yet) removes
/// nothing rather than treating that as an error.
pub(crate) fn gc_unreferenced(
    cache_root: &Path,
    referenced_digests: &BTreeSet<String>,
) -> Result<usize> {
    let sha256_dir = cache_root.join("sha256");
    let read_dir = match fs::read_dir(&sha256_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read OCI artifact cache root `{}`",
                    sha256_dir.display()
                )
            });
        }
    };

    let referenced_hex: BTreeSet<&str> = referenced_digests
        .iter()
        .map(|digest| digest_hex(digest))
        .collect();

    let mut removed = 0usize;
    for entry in read_dir {
        let entry = entry.with_context(|| {
            format!(
                "failed to enumerate OCI artifact cache root `{}`",
                sha256_dir.display()
            )
        })?;
        let file_name = entry.file_name();
        let Some(entry_digest_hex) = file_name.to_str() else {
            continue;
        };
        if referenced_hex.contains(entry_digest_hex) {
            continue;
        }
        let entry_path = entry.path();
        if entry_path.is_dir() {
            fs::remove_dir_all(&entry_path).with_context(|| {
                format!(
                    "failed to remove unreferenced OCI artifact cache entry `{}`",
                    entry_path.display()
                )
            })?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn digest_hex(expected_digest: &str) -> &str {
    expected_digest
        .strip_prefix("sha256:")
        .unwrap_or(expected_digest)
}

/// Publishes `manifest_bytes`/`layer_bytes` into `cache_dir` such that a
/// concurrent reader only ever sees either nothing or the complete,
/// internally-consistent pair of files -- never a partial write, and
/// never one file updated without the other (`Requirement: Atomic
/// Staging`, strengthened by TACH-16: the earlier version staged
/// `artifact` and its digest sidecar via two *separate* renames, so a
/// crash or a concurrent reader between them could observe a mismatched
/// pair, and its temp filename was keyed only on the process PID, which
/// two threads in this same process racing a cache miss for the same
/// digest would collide on). Both files are written into a private,
/// uniquely-named staging directory on the same filesystem as `cache_dir`
/// (so the final rename is same-filesystem and therefore atomic) and
/// fsynced before the whole directory is renamed into place in one
/// operation. Losing the rename race to another concurrent stage of this
/// exact content-addressed digest is not an error -- by definition the
/// winner's bytes are what `expected_digest` already pins, and `resolve`
/// revalidates whatever ends up on disk either way.
fn stage_cache_entry(cache_dir: &Path, manifest_bytes: &[u8], artifact_bytes: &[u8]) -> Result<()> {
    let parent = cache_dir.parent().ok_or_else(|| {
        anyhow!(
            "cache path `{}` has no parent directory",
            cache_dir.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory `{}`", parent.display()))?;

    let staging_dir = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "failed to create an OCI artifact staging directory under `{}`",
                parent.display()
            )
        })?;
    write_and_fsync(&staging_dir.path().join("manifest.json"), manifest_bytes)?;
    write_and_fsync(&staging_dir.path().join("artifact"), artifact_bytes)?;

    match fs::rename(staging_dir.path(), cache_dir) {
        Ok(()) => {
            // The directory now lives at `cache_dir`; disarm `TempDir`'s
            // own cleanup so it doesn't try to remove a path it no longer
            // owns.
            let _ = staging_dir.keep();
            Ok(())
        }
        Err(_) if cache_dir.is_dir() => {
            // Lost the rename race to another concurrent stage of this
            // exact digest. `staging_dir`'s own `Drop` cleans up our
            // now-unused copy.
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to atomically publish OCI artifact cache entry `{}` -> `{}`",
                staging_dir.path().display(),
                cache_dir.display()
            )
        }),
    }
}

fn write_and_fsync(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file =
        fs::File::create(path).with_context(|| format!("failed to create `{}`", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write `{}`", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to fsync `{}`", path.display()))?;
    Ok(())
}

/// Anonymous Docker-Registry-v2 bearer token exchange
/// (`https://<registry>/token?service=...&scope=repository:<repo>:pull`),
/// the same flow every public GHCR package pull uses. A private package
/// would need credentials this resolver does not yet supply -- out of
/// scope until Tachyon has a private Magnetar artifact to pull, per
/// `consume-magnetar-oci-artifacts`'s own task list.
fn registry_token(
    client: &reqwest::blocking::Client,
    reference: &ArtifactReference,
) -> Result<String> {
    let url = format!(
        "https://{}/token?service={}&scope=repository:{}:pull",
        reference.registry, reference.registry, reference.repository
    );
    let response = client
        .get(&url)
        .send()
        .with_context(|| {
            format!(
                "failed to reach token endpoint for `{}`",
                reference.registry
            )
        })?
        .error_for_status()
        .with_context(|| {
            format!(
                "token endpoint for `{}` rejected the request",
                reference.registry
            )
        })?;
    let parsed: RegistryTokenResponse = response
        .json()
        .context("token endpoint response was not the expected JSON shape")?;
    Ok(parsed.token)
}

fn pull(reference: &ArtifactReference) -> Result<PulledArtifact> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build OCI registry HTTP client")?;
    let token = registry_token(&client, reference)?;

    let manifest_url = format!(
        "https://{}/v2/{}/manifests/{}",
        reference.registry, reference.repository, reference.tag
    );
    let manifest_bytes = client
        .get(&manifest_url)
        .bearer_auth(&token)
        .header("Accept", "application/vnd.oci.image.manifest.v1+json")
        .send()
        .with_context(|| format!("failed to fetch OCI manifest from `{manifest_url}`"))?
        .error_for_status()
        .with_context(|| format!("registry rejected the manifest request for `{manifest_url}`"))?
        .bytes()
        .context("failed to read the manifest response body")?;

    // The trust boundary: `expected_digest` is checked against the raw
    // manifest bytes *before* anything the manifest declares (its layer's
    // digest included) is trusted for anything else. A tag repointed to a
    // different manifest, or a registry serving something else entirely,
    // is rejected right here -- never silently followed past this point.
    let actual_manifest_digest = format!("sha256:{}", sha256_hex(&manifest_bytes));
    if actual_manifest_digest != reference.expected_digest {
        bail!(
            "OCI manifest for `{}/{}:{}` has digest `{actual_manifest_digest}`, expected `{}` -- the tag \
             was repointed or the registry returned something else entirely; rejecting rather than \
             silently following it",
            reference.registry,
            reference.repository,
            reference.tag,
            reference.expected_digest,
        );
    }

    let manifest: OciManifest =
        serde_json::from_slice(&manifest_bytes).context("OCI manifest was not valid JSON")?;
    let layer = single_layer(&manifest, &reference.repository, &reference.tag)?;

    let blob_url = format!(
        "https://{}/v2/{}/blobs/{}",
        reference.registry, reference.repository, layer.digest
    );
    let blob_bytes = client
        .get(&blob_url)
        .bearer_auth(&token)
        .send()
        .with_context(|| format!("failed to fetch OCI blob from `{blob_url}`"))?
        .error_for_status()
        .with_context(|| format!("registry rejected the blob request for `{blob_url}`"))?
        .bytes()
        .context("failed to read the blob response body")?;

    let actual_layer_digest = format!("sha256:{}", sha256_hex(&blob_bytes));
    let declared_layer_digest = layer.digest.to_ascii_lowercase();
    if actual_layer_digest != declared_layer_digest {
        bail!(
            "OCI layer blob for `{}:{}` does not match its own manifest-declared digest (declared `{declared_layer_digest}`, computed `{actual_layer_digest}`)",
            reference.repository,
            reference.tag,
        );
    }

    Ok(PulledArtifact {
        manifest_bytes: manifest_bytes.to_vec(),
        layer_bytes: blob_bytes.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_well_formed_locator() {
        let reference = ArtifactReference::parse(
            "oci://ghcr.io/astorise/magnetar-component:synthetic-minimal-latest@sha256:bb393658899d36d36dda37224861c4437f249b6e240d196585a2eea17c3fe214",
        )
        .expect("well-formed locator parses");
        assert_eq!(reference.registry, "ghcr.io");
        assert_eq!(reference.repository, "astorise/magnetar-component");
        assert_eq!(reference.tag, "synthetic-minimal-latest");
        assert_eq!(
            reference.expected_digest,
            "sha256:bb393658899d36d36dda37224861c4437f249b6e240d196585a2eea17c3fe214"
        );
    }

    #[test]
    fn rejects_a_locator_with_no_digest() {
        ArtifactReference::parse(
            "oci://ghcr.io/astorise/magnetar-component:synthetic-minimal-latest",
        )
        .expect_err("a tag alone must never be accepted as a trust identity");
    }

    #[test]
    fn rejects_a_non_canonical_digest() {
        ArtifactReference::parse("oci://ghcr.io/astorise/magnetar-component:latest@sha256:not-hex")
            .expect_err("a malformed digest must be rejected at parse time, not later");
    }

    #[test]
    fn rejects_missing_prefix() {
        ArtifactReference::parse("ghcr.io/astorise/magnetar-component:latest@sha256:00")
            .expect_err("a locator missing the oci:// prefix must be rejected");
    }

    /// Builds a one-layer OCI manifest naming `layer_digest` and returns
    /// its own serialized bytes alongside its own real digest -- exactly
    /// what a locator's `expected_digest` pins in production.
    fn build_manifest(layer_digest: &str) -> (Vec<u8>, String) {
        let manifest = serde_json::json!({ "layers": [ { "digest": layer_digest } ] });
        let bytes = serde_json::to_vec(&manifest).expect("test manifest must serialize");
        let digest = format!("sha256:{}", sha256_hex(&bytes));
        (bytes, digest)
    }

    /// Stages a complete, internally-consistent cache entry (a real
    /// `manifest.json` whose declared layer digest matches `artifact_
    /// bytes`) under `cache_root`, keyed by the manifest's own real
    /// digest, and returns a reference pinning that digest.
    fn stage_valid_cache_entry(cache_root: &Path, artifact_bytes: &[u8]) -> ArtifactReference {
        let layer_digest = format!("sha256:{}", sha256_hex(artifact_bytes));
        let (manifest_bytes, manifest_digest) = build_manifest(&layer_digest);
        let cache_dir = cache_root.join("sha256").join(digest_hex(&manifest_digest));
        std::fs::create_dir_all(&cache_dir).expect("test cache dir must be creatable");
        std::fs::write(cache_dir.join("manifest.json"), &manifest_bytes)
            .expect("test manifest must be writable");
        std::fs::write(cache_dir.join("artifact"), artifact_bytes)
            .expect("test cache artifact must be writable");
        ArtifactReference {
            registry: "example.invalid".to_owned(),
            repository: "nonexistent/repo".to_owned(),
            tag: "latest".to_owned(),
            expected_digest: manifest_digest,
        }
    }

    /// GC only ever looks at directory names under `sha256/`, never at
    /// their contents, so its tests just need a directory to exist.
    fn touch_cache_dir(cache_root: &Path, digest: &str) {
        let cache_dir = cache_root.join("sha256").join(digest_hex(digest));
        std::fs::create_dir_all(&cache_dir).expect("test cache dir must be creatable");
        std::fs::write(cache_dir.join("artifact"), b"placeholder")
            .expect("test cache placeholder must be writable");
    }

    #[test]
    fn resolve_rejects_a_cached_manifest_that_does_not_hash_to_its_own_cache_key() {
        // TACH-15: the pre-fix design only ever recorded a *flat* layer
        // digest number at staging time and re-checked it against the
        // cached artifact bytes on every hit -- a coherent replacement of
        // both that number and the artifact (never re-derived from, or
        // re-tied to, an actual manifest) could pass a cache hit outright.
        // Storing the real `manifest.json` and re-deriving its digest on
        // every hit closes this: a manifest.json present under a cache
        // key but not actually hashing to that key's own expected_digest
        // -- exactly what such a replacement would produce, since forging
        // one that *does* hash to a specific pre-existing digest is a
        // SHA-256 preimage attack -- must be rejected outright, not
        // silently treated as a miss.
        let temp = tempfile::tempdir().expect("temp dir creates");
        let artifact_bytes = b"a self-consistent but never-bound artifact";
        let fabricated_layer_digest = format!("sha256:{}", sha256_hex(artifact_bytes));
        let (fabricated_manifest, _fabricated_manifest_digest) =
            build_manifest(&fabricated_layer_digest);

        // Keyed by some other, pre-existing expected_digest -- standing in
        // for whatever real manifest digest a legitimate locator actually
        // pins, which this fabricated manifest does not hash to.
        let expected_digest = format!(
            "sha256:{}",
            sha256_hex(b"the real manifest, never actually cached here")
        );
        let cache_dir = temp
            .path()
            .join("sha256")
            .join(digest_hex(&expected_digest));
        std::fs::create_dir_all(&cache_dir).expect("test cache dir must be creatable");
        std::fs::write(cache_dir.join("manifest.json"), &fabricated_manifest)
            .expect("test manifest must be writable");
        std::fs::write(cache_dir.join("artifact"), artifact_bytes)
            .expect("test cache artifact must be writable");

        let reference = ArtifactReference {
            registry: "example.invalid".to_owned(),
            repository: "nonexistent/repo".to_owned(),
            tag: "latest".to_owned(),
            expected_digest,
        };

        let error = resolve(&reference, temp.path()).expect_err(
            "a cached manifest.json that does not hash to its own cache key's expected_digest \
             must be rejected before any network access is attempted (an unreachable registry \
             host here proves no network call was made)",
        );
        assert!(error.to_string().contains("corrupted"));
    }

    #[test]
    fn resolve_rejects_a_cached_artifact_that_no_longer_matches_its_cached_manifest() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let reference = stage_valid_cache_entry(temp.path(), b"the original, correct bytes");
        let cache_dir = temp
            .path()
            .join("sha256")
            .join(digest_hex(&reference.expected_digest));
        std::fs::write(cache_dir.join("artifact"), b"tampered after staging")
            .expect("test artifact must be overwritable");

        let error = resolve(&reference, temp.path()).expect_err(
            "an artifact that no longer matches its own cached manifest's declared layer digest \
             must be rejected before any network access is attempted (an unreachable registry \
             host here proves no network call was made)",
        );
        assert!(error.to_string().contains("corrupted"));
    }

    #[test]
    fn resolve_cache_hit_takes_no_network_access() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let content = b"a real, previously-staged artifact";
        let reference = stage_valid_cache_entry(temp.path(), content);
        let cache_path = temp
            .path()
            .join("sha256")
            .join(digest_hex(&reference.expected_digest))
            .join("artifact");

        let resolved = resolve(&reference, temp.path()).expect(
            "a correctly-staged cache entry must resolve without any network access -- \
             `example.invalid` as the registry host proves it",
        );
        assert_eq!(resolved, cache_path);
    }

    #[test]
    fn stage_cache_entry_is_safe_under_concurrent_writers_of_the_same_digest() {
        // TACH-16: the pre-fix design staged `artifact` and its digest
        // sidecar via two separate renames, using a temp filename keyed
        // only on the process PID -- two threads in this same process
        // racing a cache miss for the same digest would collide on that
        // name. Driving several threads through `stage_cache_entry`
        // concurrently for the exact same cache_dir, each with the exact
        // same (content-addressed, so identical) bytes, proves the loser
        // of the rename race is treated as success rather than an error,
        // and that whatever ends up on disk afterward still validates.
        let temp = tempfile::tempdir().expect("temp dir creates");
        let artifact_bytes = b"identical content every racer stages";
        let layer_digest = format!("sha256:{}", sha256_hex(artifact_bytes));
        let (manifest_bytes, manifest_digest) = build_manifest(&layer_digest);
        let cache_dir = temp
            .path()
            .join("sha256")
            .join(digest_hex(&manifest_digest));

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let cache_dir = cache_dir.clone();
                let manifest_bytes = manifest_bytes.clone();
                let artifact_bytes = artifact_bytes.to_vec();
                std::thread::spawn(move || {
                    stage_cache_entry(&cache_dir, &manifest_bytes, &artifact_bytes)
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .expect("staging thread must not panic")
                .expect("every concurrent stage of the same digest must succeed, win or lose the rename race");
        }

        let reference = ArtifactReference {
            registry: "example.invalid".to_owned(),
            repository: "nonexistent/repo".to_owned(),
            tag: "latest".to_owned(),
            expected_digest: manifest_digest,
        };
        resolve(&reference, temp.path())
            .expect("whatever ends up on disk after the race must still pass full revalidation");
    }

    #[test]
    fn gc_removes_only_entries_absent_from_the_referenced_set() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let kept_digest = format!(
            "sha256:{}",
            sha256_hex(b"still referenced by a live binding")
        );
        touch_cache_dir(temp.path(), &kept_digest);

        let stale_digest = format!("sha256:{}", sha256_hex(b"no binding names this any more"));
        touch_cache_dir(temp.path(), &stale_digest);

        let kept_path = temp.path().join("sha256").join(digest_hex(&kept_digest));
        let stale_path = temp.path().join("sha256").join(digest_hex(&stale_digest));
        assert!(kept_path.is_dir());
        assert!(stale_path.is_dir());

        let referenced = BTreeSet::from([kept_digest.clone()]);
        let removed = gc_unreferenced(temp.path(), &referenced)
            .expect("gc over a well-formed cache root must not fail");

        assert_eq!(
            removed, 1,
            "exactly the one unreferenced entry must be removed"
        );
        assert!(
            kept_path.is_dir(),
            "an entry whose digest is still referenced must survive GC"
        );
        assert!(
            !stale_path.exists(),
            "an entry whose digest is no longer referenced by any binding must be reclaimed"
        );
    }

    #[test]
    fn gc_on_a_never_populated_cache_root_removes_nothing() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let removed = gc_unreferenced(temp.path(), &BTreeSet::new())
            .expect("a cache root with no sha256/ directory yet is not an error");
        assert_eq!(removed, 0);
    }
}
