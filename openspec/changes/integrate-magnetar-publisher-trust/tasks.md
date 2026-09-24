## 1. Trust-policy shape

- [ ] 1.1 Define a `PublisherTrustPolicy` (or extend the existing trust-store
      struct) carrying `trusted_publisher_keys: BTreeMap<String, [u8; 32]>`
      (name → raw Ed25519 public key) and `revoked_keys: BTreeSet<String>`
      (key-id fingerprints, per `magnetar_runtime::artifact_signature::key_id_for`).
- [ ] 1.2 Extend the JSON schema read from `TACHYON_COMPONENT_TRUST_STORE_ENV`
      / `TACHYON_ARTIFACT_TRUST_STORE_ENV` with optional
      `trusted_publisher_keys`/`revoked_keys` fields, additive to the existing
      `trusted_digests` field — a file with only `trusted_digests` must keep
      behaving exactly as it does today.
- [ ] 1.3 Reject a trust store file naming a revoked key under
      `trusted_publisher_keys` at parse time (fail closed on a self-
      contradictory policy file, not silently prefer one field over the other).

## 2. Wire Magnetar's verification, don't reimplement it

- [ ] 2.1 Call `magnetar_runtime::artifact_signature::verify_signature` (or
      the higher-level `ComponentTrustStore`/`ModelTrustStore` API it backs,
      whichever `a71a11a`'s public surface actually exposes for this — check
      `magnetar-runtime/src/component.rs` and `model.rs` for the intended
      call shape before wiring) from the Tachyon bridge, passing the parsed
      `trusted_publisher_keys`/`revoked_keys` and the artifact's own
      `SignatureRecord` (from its manifest) plus its actual computed digest.
- [ ] 2.2 `scripts/validate_ai_component_boundary.sh`: add a rule forbidding
      `core-host` from depending on `ed25519-dalek` (or `sha2` used for
      signature key-id derivation) directly — verification must go through
      Magnetar's API, never be duplicated.
- [ ] 2.3 Telemetry: record `authenticated_publisher` (the matched key's
      name, if any) as opaque metadata on a successful invocation, the same
      way other Magnetar-attached tags are already relayed verbatim — Tachyon
      still does not interpret it beyond passing it through.

## 3. Tests (do not require a real Magnetar publication — see proposal.md)

- [ ] 3.1 Valid signature under a trusted key verifies.
- [ ] 3.2 Signature under an unknown key is rejected (not silently trusted).
- [ ] 3.3 Malformed/invalid signature bytes are rejected.
- [ ] 3.4 Signature under a key present in `trusted_publisher_keys` but also
      listed in `revoked_keys` is rejected.
- [ ] 3.5 An artifact whose digest is explicitly pinned in
      `trusted_digests` keeps loading exactly as today, signed or not —
      digest pinning is never weakened by this change.
- [ ] 3.6 A registry/locator being "known" (e.g. resolvable, reachable) never
      grants trust by itself — only a verified signature or a pinned digest
      does.

## 4. Closure

- [ ] 4.1 `cargo test -p core-host --features ai-inference` green, including
      the new tests above.
- [ ] 4.2 `cargo clippy --workspace --all-targets --features core-host/ai-inference,core-host/legacy-wasi-nn -- -D warnings -D clippy::unwrap_used` clean.
- [ ] 4.3 `bash scripts/validate_ai_component_boundary.sh` clean, including
      the new crypto-boundary rule from 2.2.
- [ ] 4.4 Archive this change once 1–4.3 are done. This closes the
      publisher-trust half of TACH-11; the OCI-consumption half (TACH-10)
      and the end-to-end published-artifact proof (TACH-13) are the
      companion change `consume-magnetar-oci-artifacts`.
