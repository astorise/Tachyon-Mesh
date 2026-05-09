# smart-deployment-pipeline

## MODIFIED Requirements

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

### Requirement: Client-Side Deployment Bundle
The Tachyon client SHALL omit `signature` and `publicKey` from the
`manifest.yaml` inside the bundle; these fields are no longer produced by the
client.

#### Scenario: Bundle layout excludes client signing material
- **WHEN** the client builds a deployment bundle
- **THEN** the resulting `manifest.yaml` contains only `version`,
  `configPayload`, and optional `dependencies`
- **AND** it does not contain `publicKey` or `signature` fields

## ADDED Requirements

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
