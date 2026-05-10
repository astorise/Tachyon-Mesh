# smart-deployment-pipeline Specification

## Purpose
TBD - created by archiving change deployment-bundle. Update Purpose after archive.
## Requirements
### Requirement: Client-Side Deployment Bundle
The Tachyon client SHALL omit `signature` and `publicKey` from the
`manifest.yaml` inside the bundle; these fields are no longer produced by the
client.

#### Scenario: Bundle layout excludes client signing material
- **WHEN** the client builds a deployment bundle
- **THEN** the resulting `manifest.yaml` contains only `version`,
  `configPayload`, and optional `dependencies`
- **AND** it does not contain `publicKey` or `signature` fields

### Requirement: Server-Side Resolution and Locking
The core host SHALL automatically inject its own Ed25519 public key into the
`trusted_signers` list of the `IntegrityConfig` before signing and writing the
`integrity.lock`, so that subsequent reloads accept the node-signed manifest
without any manual operator step.

#### Scenario: First bundle apply self-bootstraps trusted_signers
- **GIVEN** a fresh node whose config has an empty `trusted_signers` list
- **WHEN** an admin applies a deployment bundle for the first time
- **THEN** the written `integrity.lock` contains the node's public key in
  `trusted_signers`
- **AND** the node can reload the manifest after a reboot without operator
  intervention

#### Scenario: Injection is idempotent
- **GIVEN** the node's public key is already present in `trusted_signers`
- **WHEN** a subsequent bundle apply is performed
- **THEN** `trusted_signers` contains exactly one entry for that key
- **AND** the resulting `integrity.lock` is otherwise identical to what it
  would be without the injection

### Requirement: Override Conflict Detection
The core host SHALL compare each bundled dependency that carries a local
`source` path against a persistent cluster asset-version registry using real
SemVer semantics. A conflict SHALL be reported only when the cluster registry
holds a version that is strictly greater than the bundled version AND still
satisfies the same SemVer constraint declared in the manifest's `dependencies`
block.

#### Scenario: Cluster has a strictly higher compatible version
- **GIVEN** the cluster registry records `toto = 2.4.1`
- **AND** the bundle declares `toto: { version: "^2.0.0", source: "./assets/toto.wasm" }` with bundled version `2.3.5`
- **WHEN** the host processes the bundle
- **THEN** the response status is 428
- **AND** the JSON body identifies `toto` as a conflict with `bundledVersion=2.3.5` and `clusterVersion=2.4.1`

#### Scenario: Cluster version does not satisfy the constraint
- **GIVEN** the cluster registry records `toto = 3.0.0`
- **AND** the bundle declares `toto: { version: "^2.0.0", source: "./assets/toto.wasm" }`
- **WHEN** the host processes the bundle
- **THEN** no conflict is reported (3.0.0 is outside ^2.x)
- **AND** the bundle is applied

#### Scenario: Cluster version equals the bundled version
- **GIVEN** the cluster registry records `toto = 2.3.5`
- **AND** the bundle declares the same version
- **WHEN** the host processes the bundle
- **THEN** no conflict is reported

#### Scenario: Dependency without local source is never a conflict candidate
- **GIVEN** a manifest dependency has no `source` field
- **WHEN** the host processes the bundle
- **THEN** the dependency is skipped during conflict detection regardless of what the cluster registry holds

### Requirement: Interactive Resolution Modal
Tachyon-UI SHALL intercept HTTP 428 responses from the bundle apply
flow and display a modal listing each conflict with two actions per
entry: "Use Cluster Version" (drops the local `source` so the cluster
cache version is used) and "Force Local Version" (rewrites the
constraint to strict equality `=x.y.z` matching the bundled asset).
The UI SHALL retry the bundle apply automatically once the operator
selects a resolution for every conflict.

#### Scenario: Operator chooses cluster version for a conflict
- **GIVEN** a conflict modal is shown for `toto = 2.3.5` vs cluster
  `2.4.1`
- **WHEN** the operator picks "Use Cluster Version"
- **THEN** the manifest entry for `toto` loses its `source` field
- **AND** the client rebuilds the bundle and re-POSTs it without the
  `assets/toto.wasm` payload

#### Scenario: Operator forces local version
- **WHEN** the operator picks "Force Local Version"
- **THEN** the manifest constraint becomes `=2.3.5`
- **AND** the rebuilt bundle is re-POSTed and accepted

### Requirement: Manifest Dependencies Schema
The manifest YAML SHALL accept a `dependencies` mapping where each
entry has a SemVer `version` constraint string and an optional local
`source` path pointing inside the bundle's `assets/` directory.

#### Scenario: Dependencies parse with optional source
- **WHEN** the host parses a manifest containing
  `dependencies: { toto: { version: "^2.0.0", source: "./assets/toto.wasm" } }`
- **THEN** the parser accepts the entry
- **AND** the resolver treats `toto` as eligible for the bundled
  asset path while still subject to the SemVer constraint

### Requirement: Trusted-Signer Registry
`IntegrityConfig` SHALL support an optional `trusted_signers` list of
hex-encoded Ed25519 public keys. The manifest verification step SHALL accept
any key present in the running config's `trusted_signers` list, in addition
to the key embedded in the manifest itself.

#### Scenario: Manifest signed by a trusted peer is accepted
- **GIVEN** the running config contains `trusted_signers: ["<peer-pubkey-hex>"]`
- **WHEN** a manifest carrying `<peer-pubkey-hex>` as its `publicKey` is
  verified
- **THEN** the verification succeeds without requiring the embedded
  boot-time key

#### Scenario: Manifest signed by an untrusted key is rejected
- **GIVEN** the running config's `trusted_signers` list does not contain
  a given public key
- **WHEN** a manifest carrying that key is verified
- **THEN** the verification fails with an integrity error

### Requirement: Node Public-Key Endpoint
The core host SHALL expose `GET /admin/identity/public-key` returning the
node's current Ed25519 public key in hex so that operators can add it to the
`trusted_signers` list of peer nodes.

#### Scenario: Endpoint returns the host identity key
- **GIVEN** an authenticated admin calls `GET /admin/identity/public-key`
- **THEN** the response contains a `publicKey` string equal to the hex
  encoding of the node's `HostIdentity.signing_key` verifying key

### Requirement: Asset Version Registry
`IntegrityConfig` SHALL persist a `asset_versions` map that records the
deployed SemVer string of each bundled asset after a successful bundle apply.
The map SHALL be updated inside the signed `integrity.lock` so it survives
reloads and propagates via gossip.

#### Scenario: Successful apply updates the registry
- **GIVEN** a bundle declares `toto: { version: "^2.3.5", source: "..." }`
- **WHEN** the host applies the bundle without conflicts
- **THEN** the written `integrity.lock` contains `asset_versions: { "toto": "2.3.5" }`
- **AND** a subsequent bundle apply for `toto` uses this entry as the cluster baseline

