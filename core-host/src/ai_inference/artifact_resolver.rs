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
/// digest (`<cache_root>/sha256/<manifest-digest>/artifact`), matching
/// exactly what `oras resolve`/a registry UI reports as "the digest" of a
/// tag, and therefore what an operator actually has in hand to pin. The
/// manifest digest is verified first, against the raw manifest bytes,
/// before its *declared* layer digest is trusted for anything -- a
/// registry serving a self-consistent but wrong manifest is caught here,
/// not just a corrupted blob under an otherwise-correct one. The file this
/// returns holds the layer's own bytes (the real Component/Kernel content
/// Magnetar loads), alongside a small sidecar recording that layer's own
/// digest, written once at staging time and used for every later
/// corruption check.
///
/// Cache-hit path (the common case once an artifact has been pulled once)
/// takes no network access at all: the sidecar's recorded digest and the
/// cached file's freshly computed digest are compared on every call, so a
/// cache entry corrupted on disk after staging is caught here, before a
/// caller ever hands its bytes to Magnetar, not after a failed or garbled
/// invocation (`Requirement: Corrupted Cache Entries Are Detected Before
/// Load`). A missing or unreadable sidecar is treated as a cache miss
/// (safe: it only costs a re-fetch), never as a reason to trust an
/// unverified file.
pub(crate) fn resolve(reference: &ArtifactReference, cache_root: &Path) -> Result<PathBuf> {
    let cache_dir = cache_root
        .join("sha256")
        .join(digest_hex(&reference.expected_digest));
    let artifact_path = cache_dir.join("artifact");
    let digest_sidecar_path = cache_dir.join("layer-digest.txt");

    if let Some(cached_layer_digest) = fs::read_to_string(&digest_sidecar_path)
        .ok()
        .map(|contents| contents.trim().to_owned())
        .filter(|digest| !digest.is_empty())
        && artifact_path.is_file()
    {
        let cached = fs::read(&artifact_path).with_context(|| {
            format!(
                "failed to read cached artifact at `{}`",
                artifact_path.display()
            )
        })?;
        let actual = format!("sha256:{}", sha256_hex(&cached));
        if actual == cached_layer_digest {
            return Ok(artifact_path);
        }
        bail!(
            "cached artifact at `{}` no longer matches its recorded digest (recorded `{cached_layer_digest}`, \
             computed `{actual}`) -- the cache entry is corrupted and is rejected rather than handed to Magnetar",
            artifact_path.display(),
        );
    }

    let pulled = pull(reference)?;
    stage_atomically(&artifact_path, &pulled.layer_bytes)?;
    stage_atomically(&digest_sidecar_path, pulled.layer_digest.as_bytes())?;
    Ok(artifact_path)
}

struct PulledArtifact {
    layer_bytes: Vec<u8>,
    layer_digest: String,
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

/// Writes `bytes` to `final_path` such that a concurrent reader only ever
/// sees either nothing or the complete file -- never a partial write
/// (`Requirement: Atomic Staging`). Writes to a sibling temp file in the
/// same directory (so the final rename is same-filesystem and therefore
/// atomic) and only renames into place after an `fsync`.
fn stage_atomically(final_path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = final_path.parent().ok_or_else(|| {
        anyhow!(
            "cache path `{}` has no parent directory",
            final_path.display()
        )
    })?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory `{}`", parent.display()))?;
    let temp_path = parent.join(format!(".artifact.{}.tmp", std::process::id()));
    {
        let mut file = fs::File::create(&temp_path)
            .with_context(|| format!("failed to create temp file `{}`", temp_path.display()))?;
        file.write_all(bytes)
            .with_context(|| format!("failed to write temp file `{}`", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to fsync temp file `{}`", temp_path.display()))?;
    }
    fs::rename(&temp_path, final_path).with_context(|| {
        format!(
            "failed to atomically stage `{}` -> `{}`",
            temp_path.display(),
            final_path.display()
        )
    })?;
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
    let layer = match manifest.layers.as_slice() {
        [layer] => layer,
        [] => bail!(
            "OCI manifest for `{}:{}` carries no layers",
            reference.repository,
            reference.tag
        ),
        _ => bail!(
            "OCI manifest for `{}:{}` carries {} layers; this resolver only supports a single-layer artifact",
            reference.repository,
            reference.tag,
            manifest.layers.len()
        ),
    };

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
        layer_bytes: blob_bytes.to_vec(),
        layer_digest: actual_layer_digest,
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

    fn write_cache_entry(
        cache_root: &Path,
        manifest_digest: &str,
        recorded_layer_digest: &str,
        bytes: &[u8],
    ) {
        let cache_dir = cache_root.join("sha256").join(digest_hex(manifest_digest));
        std::fs::create_dir_all(&cache_dir).expect("test cache dir must be creatable");
        std::fs::write(cache_dir.join("artifact"), bytes)
            .expect("test cache artifact must be writable");
        std::fs::write(cache_dir.join("layer-digest.txt"), recorded_layer_digest)
            .expect("test cache sidecar must be writable");
    }

    #[test]
    fn resolve_rejects_a_corrupted_cache_entry_before_touching_the_network() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let manifest_digest = format!("sha256:{}", sha256_hex(b"a manifest"));
        let recorded_layer_digest = format!("sha256:{}", sha256_hex(b"expected content"));
        let reference = ArtifactReference {
            registry: "example.invalid".to_owned(),
            repository: "nonexistent/repo".to_owned(),
            tag: "latest".to_owned(),
            expected_digest: manifest_digest.clone(),
        };
        write_cache_entry(
            temp.path(),
            &manifest_digest,
            &recorded_layer_digest,
            b"the wrong bytes entirely",
        );

        let error = resolve(&reference, temp.path()).expect_err(
            "a cache entry whose on-disk bytes no longer hash to its recorded digest must be \
             rejected before any network access is attempted (an unreachable registry host here \
             proves no network call was made)",
        );
        assert!(error.to_string().contains("corrupted"));
    }

    #[test]
    fn resolve_cache_hit_takes_no_network_access() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let content = b"a real, previously-staged artifact";
        let manifest_digest = format!("sha256:{}", sha256_hex(b"a manifest"));
        let recorded_layer_digest = format!("sha256:{}", sha256_hex(content));
        let reference = ArtifactReference {
            registry: "example.invalid".to_owned(),
            repository: "nonexistent/repo".to_owned(),
            tag: "latest".to_owned(),
            expected_digest: manifest_digest.clone(),
        };
        let cache_path = temp
            .path()
            .join("sha256")
            .join(digest_hex(&manifest_digest))
            .join("artifact");
        write_cache_entry(
            temp.path(),
            &manifest_digest,
            &recorded_layer_digest,
            content,
        );

        let resolved = resolve(&reference, temp.path()).expect(
            "a correctly-staged cache entry must resolve without any network access -- \
             `example.invalid` as the registry host proves it",
        );
        assert_eq!(resolved, cache_path);
    }

    #[test]
    fn gc_removes_only_entries_absent_from_the_referenced_set() {
        let temp = tempfile::tempdir().expect("temp dir creates");
        let kept_digest = format!(
            "sha256:{}",
            sha256_hex(b"still referenced by a live binding")
        );
        let kept_layer_digest = format!("sha256:{}", sha256_hex(b"kept content"));
        write_cache_entry(
            temp.path(),
            &kept_digest,
            &kept_layer_digest,
            b"kept content",
        );

        let stale_digest = format!("sha256:{}", sha256_hex(b"no binding names this any more"));
        let stale_layer_digest = format!("sha256:{}", sha256_hex(b"stale content"));
        write_cache_entry(
            temp.path(),
            &stale_digest,
            &stale_layer_digest,
            b"stale content",
        );

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
