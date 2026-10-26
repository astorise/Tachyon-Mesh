## ADDED Requirements

### Requirement: Component and Model Artifact trust MAY be established by publisher key, not just by digest
Tachyon's trust-store file(s) (named by `TACHYON_COMPONENT_TRUST_STORE`/
`TACHYON_ARTIFACT_TRUST_STORE`) MAY additionally carry `trusted_publisher_keys`
(publisher name → hex-encoded Ed25519 public key) and `revoked_keys` (key-id
fingerprints), both optional and additive to the existing `trusted_digests`
field — a file naming only `trusted_digests` SHALL behave exactly as it did
before this capability existed. Tachyon SHALL pass this policy data to
Magnetar's own trust API (`ArtifactTrustPolicy::trust_publisher_key`/
`trust_component_publisher_key`/`revoke_key`/`revoke_component_key`) and
SHALL NOT perform Ed25519 signature verification or key-fingerprint
derivation itself — that stays entirely on Magnetar's side of the API.
Digest pinning remains the floor: an artifact whose digest is explicitly
pinned keeps loading exactly as it always has, whether or not it is also
signed.

#### Scenario: A valid signature under a trusted publisher key authenticates an artifact with no digest pinned
- **WHEN** a Component or Model Artifact carries a signature that verifies
  under a key named in `trusted_publisher_keys`
- **AND** neither the Component nor its Model Artifact digest is pinned in
  `trusted_digests`
- **THEN** Tachyon authorizes the load on the strength of the verified
  signature alone

#### Scenario: A signature under an unknown key never grants trust
- **WHEN** an artifact carries a signature under a key that `trusted_
  publisher_keys` never names
- **THEN** Tachyon rejects the binding
- **AND** the artifact being resolvable or reachable on disk never
  substitutes for a verified signature or a pinned digest

#### Scenario: A malformed or invalid signature under a trusted key is rejected
- **WHEN** an artifact carries a signature that is syntactically present but
  does not cryptographically verify under a key `trusted_publisher_keys`
  does name
- **THEN** Tachyon rejects the binding exactly as it would an unsigned,
  untrusted artifact

#### Scenario: A trust-store file naming the same key as both trusted and revoked is rejected at parse time
- **WHEN** a trust-store file lists the same key-id fingerprint under both
  `trusted_publisher_keys` and `revoked_keys`
- **THEN** Tachyon rejects the file itself as self-contradictory before any
  artifact is evaluated against it
- **AND** this failure happens at trust-store parse time, independent of
  whether any artifact in a given load actually carries a signature under
  that key

#### Scenario: Digest pinning is never weakened by the presence of publisher-key trust
- **WHEN** an artifact's digest is explicitly pinned in `trusted_digests`
- **AND** that same artifact also carries a signature that does not verify
  under any trusted key
- **THEN** Tachyon still authorizes the load on the strength of the pinned
  digest alone, unaffected by the unverifiable signature
