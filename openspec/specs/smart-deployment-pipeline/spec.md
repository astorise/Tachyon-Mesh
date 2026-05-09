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
The core host SHALL sign the `integrity.lock` produced by a bundle apply with
the receiving node's own Ed25519 `HostIdentity` key instead of delegating
signing authority to the client workstation.

#### Scenario: Bundle manifest requires no client signature
- **GIVEN** the client POSTs a bundle whose `manifest.yaml` contains only
  `configPayload` and optional `dependencies`
- **WHEN** the host processes the bundle
- **THEN** the host signs the `configPayload` with its own `HostIdentity.signing_key`
- **AND** the written `integrity.lock` carries the node's public key, not the
  client's

#### Scenario: Peer nodes can trust the signing node
- **GIVEN** node A's public key has been added to the `trusted_signers` list of
  node B's sealed config
- **WHEN** node B verifies a manifest signed by node A via gossip
- **THEN** the verification succeeds
- **AND** node B accepts and applies the manifest

### Requirement: Override Conflict Detection
The core host SHALL detect when a bundled asset's pinned version is
shadowed by a strictly higher cluster-cached version that also
satisfies the manifest constraint, and SHALL respond with HTTP 428
including a JSON `conflicts` list naming each shadowed dependency,
its bundled version, and the cluster version.

#### Scenario: Bundle with override returns 428
- **GIVEN** the bundle pins `toto = 2.3.5` under a `^2.0.0`
  constraint and the cluster cache holds `toto = 2.4.1`
- **WHEN** the host processes the bundle
- **THEN** the response status is 428
- **AND** the JSON body contains a `conflicts` entry with
  `name=toto`, `bundled_version=2.3.5`, and `cluster_version=2.4.1`

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

