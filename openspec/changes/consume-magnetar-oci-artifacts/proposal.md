## Why

The 2026-09-24 Tachyon audit (TACH-10, TACH-13) found that Magnetar `main`
now has a real OCI publication contract for Component and Kernel Exchange
Bundle artifacts (`release-publication.yml`, `component-artifact-distribution.yml`,
`tools/release-publish`, OCI media types, checksum/SBOM/provenance) — but
Tachyon's inference path only ever resolves a `magnetar:<local path>` binding
against a directory already on disk. There is no resolver, no registry
credential handling, no digest-verified pull, and no content-addressed local
cache anywhere in `core-host`.

**This change is explicitly blocked**, not just scoped: as of the audit, no
stable Magnetar GitHub Release/OCI artifact was yet observable — the
publication workflow exists on Magnetar `main` but a first real publish was
still described as pending manual trigger/configuration. Nothing here should
be implemented against a *simulated* artifact; the audit's own closure
criterion (TACH-13) is an end-to-end test against something Magnetar actually
published. Treat this file as the design to pick up once that exists, not a
green light to start now.

## What Changes

- Add an `ArtifactReference` type: `{ locator: oci://..., expected_digest:
  sha256:... }`. The locator is never itself a trust decision — see below.
- Add a Tachyon-owned `ArtifactResolver` that:
  - authenticates to the registry (credentials, exactly like the existing
    upstream OpenAI-scheme credential handling already does for remote
    providers — same pattern, different protocol);
  - pulls the manifest/layers for `locator`;
  - verifies the pulled content's actual digest equals `expected_digest`
    *before* anything downstream sees it — a locator that resolves to an
    unexpected digest is rejected outright, never silently accepted because
    "the tag still points somewhere";
  - stages the verified bytes into a local content-addressed cache
    (`/var/lib/tachyon/artifacts/sha256/<digest>/` or equivalent), atomically
    (no partial/torn writes visible to a concurrent loader);
  - hands `MagnetarRuntime::try_load` the resulting local immutable
    directory path — exactly the same `magnetar:<path>` shape it already
    accepts. **`MagnetarRuntime` itself gains no new surface**: it must not
    grow `pull_from_ghcr`/`login_registry`/`resolve_tag` methods. Transport
    and location resolution stay entirely on the Tachyon side of the
    boundary; Magnetar keeps trusting and loading a local directory the way
    it always has.
- Cache lifecycle: hit-without-network-access when the digest is already
  staged, garbage collection for unreferenced digests, and correctness under
  a corrupted-on-disk cache entry (detected before load, not after a bad
  invocation).
- A repointed tag that now resolves to a different digest than
  `expected_digest` must be rejected, not silently followed — this is the
  same "tag is never a trust identity" principle the publisher-trust change
  (`integrate-magnetar-publisher-trust`) establishes for signatures.

## Impact

- Affected specs: `ai-inference` (artifact resolution section, new).
- Affected code: new `core-host/src/ai_inference/artifact_resolver.rs` (or
  similar), `core-host/src/ai_inference/magnetar_runtime.rs` (caller only —
  it receives a local path, exactly as today), `core-host/Cargo.toml` (new
  optional dependency for OCI registry client + auth, gated behind a new
  feature so builds that only use local `magnetar:` paths pay nothing extra).
- Affected behavior: additive — existing `magnetar:<local path>` bindings
  are untouched. A new `oci://` (or similar) locator scheme becomes usable
  once this lands.
- **Hard blocker**: TACH-13's end-to-end proof (publish → pull → digest →
  signature/trust → stage → invoke) needs bytes and digests that only a real
  Magnetar publication can provide. Do not fabricate a stand-in registry for
  this proof — that would validate the code path against fixtures the real
  path will never see (this project's own dead_code/architecture guards
  exist precisely to keep fixtures from quietly becoming the only thing
  ever tested). Wait for the artifact, or ask before proceeding without one.
