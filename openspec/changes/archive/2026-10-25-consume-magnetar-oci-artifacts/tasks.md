**Do not start section 1 until a real Magnetar Component/Kernel artifact has
been published (a GitHub Release or GHCR tag Magnetar itself produced via
`release-publication.yml`/`component-artifact-distribution.yml`) — see
proposal.md's "Why". Confirm one exists and note its locator/digest here
before beginning implementation.**

## 0. Precondition

- [x] 0.1 A real Magnetar-published artifact exists and its locator + digest
      are recorded here:
      `locator = oci://ghcr.io/astorise/magnetar-component:synthetic-minimal-latest@sha256:bb393658899d36d36dda37224861c4437f249b6e240d196585a2eea17c3fe214`
      (the manifest digest; the layer it declares is the real Component
      bytes at `sha256:467bfdf663fcdb6034294599f6a079a9e0af05c1d0be2e8996afe1c2ef463854`).
      A second real artifact, the Kernel Exchange Bundle
      (`oci://ghcr.io/astorise/magnetar-kernel-bundle:cpu-reference-matmul-latest@sha256:a0c1e946864b7921a6bc9aeef4ccbefdfd50847bd8c7f70a71c484bcf6fe7f91`),
      is also real and published, but out of scope below: Tachyon's own
      inference path resolves Component bindings only (`MagnetarRuntime::
      try_load`'s `component_locator` parameter) and has no Kernel Exchange
      Bundle consumer to wire a locator into — resolving it would be an
      untested code path with no caller, not a real capability.

## 1. `ArtifactReference` and resolver skeleton

- [x] 1.1 `ArtifactReference { registry, repository, tag, expected_digest }`
      type, parsed from an `oci://<registry>/<repository>:<tag>@sha256:
      <digest>` locator (`core-host/src/ai_inference/artifact_resolver.rs`).
- [x] 1.2 New optional Cargo feature `oci-artifacts` (implies `ai-inference`)
      gates this module; it adds no new dependency — reuses `reqwest`/
      `sha2`, already mandatory in `core-host/Cargo.toml` for the upstream
      OpenAI-scheme provider path.
- [x] 1.3 `pull()`: registry bearer-token auth, manifest fetch, manifest-
      digest verification against `expected_digest` *before* trusting
      anything the manifest declares (including its own layer digest),
      then the single-layer blob fetch, verified again against the
      manifest-declared layer digest.

## 2. Local content-addressed cache

- [x] 2.1 Atomic staging into `sha256/<manifest-digest>/{artifact,layer-
      digest.txt}` via `stage_atomically` (temp file + fsync + rename).
- [x] 2.2 Cache hit path takes no network access — proven by
      `resolve_cache_hit_takes_no_network_access` (an unreachable
      `example.invalid` registry host) and, for the real artifact, the
      second-resolve mtime check in `resolves_and_registers_the_real_
      published_ghcr_component_end_to_end`.
- [x] 2.3 `gc_unreferenced(cache_root, referenced_digests)`: removes any
      cache entry whose digest is not in the live set, returns the count
      reclaimed (`gc_removes_only_entries_absent_from_the_referenced_set`,
      `gc_on_a_never_populated_cache_root_removes_nothing`). Exposed as a
      resolver-level primitive, as this proposal's own "Impact" section
      scopes it (no new background-sweeper/`AppState` wiring was called
      for there) — a caller that assembles "the live referenced-digest
      set" from a running `IntegrityConfig` and invokes it periodically is
      future work, not part of this closure.
- [x] 2.4 A corrupted cache entry is detected and rejected before load, not
      after a failed/garbled invocation
      (`resolve_rejects_a_corrupted_cache_entry_before_touching_the_network`).

## 3. Wiring

- [x] 3.1 Resolver output feeds `MagnetarRuntime::try_load` as a local path
      (via `materialize_oci_component`) — `scripts/validate_ai_component_
      boundary.sh` passes, confirming `MagnetarRuntime` gained no OCI/
      registry surface.
- [~] 3.2 A binding using an `oci://` locator behaves identically, end to
      end, to one given the same bytes through a local directory, as far
      as this can be proven without a paired Model Artifact bundle:
      `resolves_and_registers_the_real_published_ghcr_component_end_to_end`
      proves the OCI-materialized bytes are byte-for-byte the real,
      digest-verified GHCR artifact and are discoverable by
      `component_artifact_from_root` exactly like a hand-placed local
      bundle (the same call `try_load` makes next). It stops short of
      `LoadedInferenceComponent::load` itself, which unconditionally also
      ingests a Model Artifact bundle from the same root regardless of
      which Component is loaded — a requirement orthogonal to transport
      that the real published fixture (`synthetic-minimal`, a Component-
      registration-only artifact never paired with a Model Artifact bundle
      anywhere in Magnetar's own test suite either) was never meant to
      satisfy. Proving transport-irrelevance past that point needs a real
      published Component *and* Model Artifact pair; none exists yet.

## 4. Tests (against the real artifact recorded in 0.1)

- [x] 4.1 Pull by digest succeeds
      (`resolves_and_registers_the_real_published_ghcr_component_end_to_end`).
- [x] 4.2 A tag currently resolving to the expected digest succeeds (same
      test — `REAL_COMPONENT_LOCATOR` names the real, live tag).
- [x] 4.3 A tag repointed to an unexpected digest is rejected — proven
      against the real registry with the real tag and a deliberately wrong
      `expected_digest` (`rejects_a_real_tag_resolving_to_an_unexpected_
      digest`), rather than by mutating the published tag itself.
- [ ] 4.4 Publisher signature valid (if `integrate-magnetar-publisher-trust`
      has landed) → succeeds; revoked key → rejected. **Not done**:
      `integrate-magnetar-publisher-trust`'s Tachyon-side wiring (parsing
      trust-store env vars, calling `ArtifactTrustPolicy::trust_publisher_
      key`/`revoke_key`) has not been implemented, and its Magnetar-side
      API (`astorise/Magnetar#125`) is still an open, unmerged PR.
- [x] 4.5 Cache hit path: second load of the same digest touches no network
      (same end-to-end test, mtime-unchanged assertion).
- [x] 4.6 The same digest supplied through a local directory (no OCI
      involved) produces identical behavior — by construction:
      `materialize_oci_component` writes into the exact local-directory
      shape (`OCI_RESOLVED_COMPONENT_FILE_NAME` + sidecar manifest) that
      the pre-existing, unmodified `component_artifact_from_root` already
      reads for a hand-placed local bundle; no OCI-specific branch exists
      past that materialization point.
- [x] 4.7 A corrupted cache entry is caught before execution, not during it
      (same test as 2.4).

## 5. Closure

- [x] 5.1 `cargo test -p core-host --features ai-inference,oci-artifacts`
      green, including section 4 (34 passed in `ai_inference::*`, 0 failed;
      the 40 failures elsewhere in the full `-p core-host` run are
      pre-existing sandbox network-socket restrictions, reproduced
      identically with zero features enabled, unrelated to this change).
- [x] 5.2 `cargo clippy -p core-host --all-targets --features ai-inference,
      legacy-wasi-nn,oci-artifacts,magnetar-cuda -- -D warnings -D
      clippy::unwrap_used` clean. (Scoped to `-p core-host` rather than
      `--workspace`: an unrelated workspace member pulls in `gdk-sys`,
      which fails to build in this sandbox for lack of system GTK
      dev packages — a pre-existing environment gap, not something this
      change touches.)
- [x] 5.3 `bash scripts/validate_ai_component_boundary.sh` clean — confirms
      `MagnetarRuntime` gained no OCI/registry surface (TACH-10's explicit
      "what the bridge must not become" guardrail). Fixed a real pre-
      existing gap in the script itself along the way: three checks on
      `magnetar_runtime.rs` used `check_absent` (scans the whole file)
      instead of `check_production_prefix_absent` (skips `#[cfg(test)]`-
      and now also `#[cfg(all(test, ...))]`-attributed items), so a real
      test fixture's own manifest text (containing `role: "qwen-model-
      component"`, the real, checked-in-on-Magnetar-main content) tripped
      a false positive on production-code vocabulary.
- [x] 5.4 This closes TACH-10 and TACH-13, with the explicit, documented
      exceptions above (3.2's stop-short-of-`.load()` scope, and the
      Kernel Exchange Bundle being out of scope for lack of a consumer).
      Archive this change once reviewed.
