# Proposal: Host-Signed Deployment Bundles

## Why

The current bundle apply flow forwards the client-signed manifest (carrying
the configurator's Ed25519 public key) to the host, which merely verifies the
external signature. This means the trust root is the workstation key, not the
node key, and peers that receive the resulting `integrity.lock` via gossip must
also know the configurator key.

Every Tachyon node already holds its own Ed25519 `HostIdentity.signing_key`
used for route claims and gossip events. Using it to sign the `integrity.lock`
produced by a bundle apply is the natural extension: the trust root becomes the
node itself, rotating with the node's identity rather than the operator's
workstation.

## What Changes

- The client bundle manifest (`manifest.yaml`) MUST NOT include a `signature`
  or `publicKey` field; only a `configPayload` JSON block and an optional
  `dependencies` section are required.
- `admin_manifest_bundle_handler` signs the `configPayload` with the receiving
  node's `HostIdentity.signing_key` to produce the `integrity.lock` instead of
  delegating back to the client.
- `IntegrityConfig` gains an optional `trusted_signers` list (hex-encoded
  Ed25519 public keys). When non-empty, `verify_integrity_payload` accepts any
  key in the list in addition to the embedded boot-time key. This enables peer
  nodes to trust manifests signed by a different cluster member.
- A new `PUT /admin/identity/trusted-signers` endpoint allows an authenticated
  admin to append the current node's public key to the `trusted_signers` in the
  running config, which is then sealed and gossiped.

## Non-goals

- Full PKI or CA-signed node certificates (remains future work).
- Automatic rotation of the host signing key.
- Support for multiple concurrent cluster CAs.
