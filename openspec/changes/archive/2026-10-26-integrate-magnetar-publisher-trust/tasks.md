## 1. Trust-policy shape

- [x] 1.1 No new `PublisherTrustPolicy` struct was needed: Magnetar's own
      `ArtifactTrustPolicy::trust_publisher_key`/`trust_component_publisher_key`/
      `revoke_key`/`revoke_component_key` (added in `astorise/Magnetar#125`,
      repinned here) already carry the `VerifyingKeyBytes`/`PublisherIdentity`/
      key-id shapes this task described — Tachyon's own job is purely to
      parse the JSON file and call them (`core-host/src/ai_inference/
      magnetar_runtime.rs::apply_trust_store`).
- [x] 1.2 `apply_trust_store` reads `trusted_publisher_keys` (object: name →
      hex-encoded 32-byte Ed25519 public key) and `revoked_keys` (array of
      key-id fingerprint strings) from the same file `trusted_digests`
      already uses, both optional and additive — a file with only
      `trusted_digests` is byte-for-byte unchanged in behavior (the
      existing `trusted_digests`-parsing code path was not touched).
- [x] 1.3 `apply_trust_store` computes `key_id_for` on every parsed
      `trusted_publisher_keys` entry and rejects the whole file at parse
      time (`bail!`) if that key id also appears in `revoked_keys` —
      before Magnetar's trust evaluation ever sees the file.

## 2. Wire Magnetar's verification, don't reimplement it

- [x] 2.1 `apply_trust_store` calls `ArtifactTrustPolicy::trust_publisher_key`/
      `trust_component_publisher_key`/`revoke_key`/`revoke_component_key`
      directly (the actual public surface `astorise/Magnetar#125` added —
      narrower and simpler than the originally-guessed `verify_signature`/
      `SignatureRecord` call shape this task was written against before
      that PR's real API existed). No Ed25519 code of any kind is called,
      imported, or reimplemented in `core-host`.
- [x] 2.2 `scripts/validate_ai_component_boundary.sh` gained two rules:
      `core-host/src/ai_inference.rs` and `core-host/src/ai_inference/
      magnetar_runtime.rs` may not import `ed25519-dalek` (production code
      only — test fixtures that sign with real keys to prove the wiring,
      the same way `astorise/Magnetar#125`'s own test suite does, are
      exempt), and `magnetar_runtime.rs` additionally may not call
      `Sha256::digest`/import `sha2` at all in production code, closing the
      specific "reimplement `key_id_for` by hand" risk this task named.
      Verified the rule actually fires (temporarily injected a bare
      `use ed25519_dalek::...;`, confirmed the script failed, then
      reverted) before trusting it.
- [ ] 2.3 **Not done, real gap found while implementing**: `magnetar_
      inference_component`'s only public entry point for a Component load,
      `LoadedInferenceComponent::load`, returns no information about which
      key (if any) authenticated the artifact — that information
      (`ComponentTrustDecision::authenticated_publisher`) exists inside
      Magnetar's own `ComponentTrustStore::evaluate`, but `evaluate`
      itself, `register_inference_component_artifact`, and
      `ComponentTrustDecision` are all crate-private to
      `inference-components`/`magnetar-runtime`, not reachable from
      `core-host` (confirmed the same way TACH-10's own private-API check
      was confirmed: direct calls fail with `E0603`/`E0616`-class privacy
      errors). Tachyon cannot record `authenticated_publisher` as
      telemetry without Magnetar first exposing it on `LoadedInference
      Component::load`'s own return value or an equivalent public accessor
      — a further Magnetar-side API change, analogous to how 2.1's actual
      call shape needed `astorise/Magnetar#125` to exist first. Filed as a
      follow-up; not blocking the rest of this change, since the actual
      trust *decision* (accept/reject) is fully wired and tested without it.

## 3. Tests (do not require a real Magnetar publication — see proposal.md)

All of the following drive a real Ed25519 signature (via `ed25519-dalek`
test keys, the same technique `astorise/Magnetar#125`'s own test suite
uses) through the full `load_binding` -> `MagnetarRuntime::try_load` ->
`LoadedInferenceComponent::load` path against the real, checked-in Qwen
Component fixture — not a mock, and not a call to any Magnetar-internal
function directly. `core-host/src/ai_inference.rs::tests::
publisher_trust_tests`.

- [x] 3.1 `a_valid_signature_under_a_trusted_publisher_key_authenticates_with_no_digest_pinned`.
- [x] 3.2 `a_signature_under_an_unknown_key_is_rejected`.
- [x] 3.3 `malformed_signature_bytes_under_a_trusted_key_are_rejected`.
- [x] 3.4 `a_key_present_in_both_trusted_and_revoked_is_rejected` — caught by
      this change's own parse-time guard (task 1.3) rather than deferred to
      Magnetar's runtime evaluation; either layer rejecting it satisfies
      the actual requirement ("is rejected"), and Magnetar's own
      evaluation-time "revocation wins" behavior is independently proven
      by `astorise/Magnetar#125`'s own test suite, not duplicated here.
- [x] 3.5 `digest_pinning_keeps_working_alongside_an_unverifiable_signature`.
- [x] 3.6 Not a new test: already proven by the pre-existing
      `explicit_magnetar_binding_rejects_untrusted_qwen_bundle` (a fully
      resolvable, real, on-disk Component is still rejected absent
      explicit trust) and by `consume-magnetar-oci-artifacts`'s
      `rejects_a_real_tag_resolving_to_an_unexpected_digest` (a reachable
      OCI tag is still rejected when it resolves to an unexpected digest)
      — a locator being reachable has never been sufficient for trust in
      this codebase, before or after this change.

## 4. Closure

- [x] 4.1 `cargo test -p core-host --features ai-inference,oci-artifacts`
      green: 40/40 `ai_inference::` tests (0 regressions), including all of
      section 3 above.
- [x] 4.2 `cargo clippy -p core-host --all-targets --features ai-inference,
      legacy-wasi-nn,oci-artifacts,magnetar-cuda -- -D warnings -D
      clippy::unwrap_used` clean. (Scoped to `-p core-host` rather than
      `--workspace` for the same pre-existing, unrelated `gdk-sys`/system-
      GTK sandbox gap `consume-magnetar-oci-artifacts` already documented.)
- [x] 4.3 `bash scripts/validate_ai_component_boundary.sh` clean, including
      the two new rules from 2.2.
- [x] 4.4 Archive this change. This closes the publisher-trust half of
      TACH-11 (with the documented 2.3 telemetry gap above as an explicit,
      non-blocking exception); the OCI-consumption half (TACH-10) and the
      end-to-end published-artifact proof (TACH-13) were closed separately
      by the companion change `consume-magnetar-oci-artifacts`.
