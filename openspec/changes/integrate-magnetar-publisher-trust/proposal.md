## Why

The 2026-09-24 Tachyon audit (TACH-06, TACH-11) found that Magnetar `main`
(`a71a11a`) has landed Ed25519 signature verification, `trusted_publisher_keys`,
and `revoked_keys` for both Component and Model Artifact trust
(`magnetar-runtime/src/artifact_signature.rs`, `docs/cryptographic-artifact-signatures.md`),
but the Tachyon bridge (`core-host/src/ai_inference/magnetar_runtime.rs`) only
ever loads a flat `{"trusted_digests": [...]}` JSON file and applies it through
`ArtifactTrustPolicy::trust_component_digest`/`trust_digest`. Digest pinning is
sound and fail-closed on its own, but Tachyon cannot yet express or verify a
publisher identity, cannot revoke a compromised key, and gains none of the
rotation story Magnetar's new trust contract makes possible.

Tachyon must stay the owner of *host trust policy* — which keys are trusted,
which are revoked, for which alias — while letting Magnetar do the actual
Ed25519 verification it already implements. This change does not touch how
artifacts are *transported*; see the companion change
`consume-magnetar-oci-artifacts` for that. It only extends what Tachyon can
declare about who it trusts to have signed an artifact it already has bytes
for (local directory or, once that companion change lands, a resolved OCI
pull).

## What Changes

- Repin `vendor/Magnetar` past `a71a11a` (done as of TACH-06) — this proposal
  begins consuming the trust API it introduced, not just building against it.
- Add a host-side trust-policy shape carrying `trusted_publisher_keys`
  (name → Ed25519 public key hex) and `revoked_keys` (key-id/fingerprint set),
  parsed from the same environment-pointed trust store file(s) already used
  for `TACHYON_COMPONENT_TRUST_STORE`/`TACHYON_ARTIFACT_TRUST_STORE`, so this
  is additive to the existing file format rather than a new mechanism.
- Pass those two collections into Magnetar's own
  `magnetar_runtime::artifact_signature`/`component`/`model` trust evaluation
  APIs (`verify_signature`, `key_id_for`, the trust-store types in
  `magnetar-runtime/src/component.rs` and `model.rs`) — Tachyon supplies
  policy data, Magnetar does the cryptographic verification. No Ed25519 code
  is duplicated in `core-host`.
- Extend `scripts/validate_ai_component_boundary.sh` with a rule forbidding
  `core-host` from importing `ed25519-dalek` or any signature-primitive crate
  directly, so this boundary stays enforced the same way TACH-01/02 already
  are for Provider/generate vocabulary.
- Keep digest pinning as the floor: an artifact whose digest is explicitly
  pinned in the trust store keeps today's behavior unconditionally, whether
  or not it is also signed — this change is additive, not a replacement of
  the existing trust path.

## Impact

- Affected specs: `ai-inference` (trust policy section).
- Affected code: `core-host/src/ai_inference/magnetar_runtime.rs`,
  `core-host/src/ai_inference.rs` (trust store env parsing), `scripts/validate_ai_component_boundary.sh`.
- Affected behavior: none by default — a deployment that does not populate
  `trusted_publisher_keys`/`revoked_keys` sees no change. A deployment that
  does gains publisher-identity trust and key revocation on top of digest
  pinning.
- Blocked on: nothing for the trust-policy plumbing itself, but the *proof*
  tests in `tasks.md` (valid/unknown/invalid/revoked key, registry-does-not-
  imply-trust) need real signed fixtures, which are cheap to generate locally
  with `ed25519-dalek` test keys — this does **not** require a real Magnetar
  publication (unlike `consume-magnetar-oci-artifacts`, which does).
