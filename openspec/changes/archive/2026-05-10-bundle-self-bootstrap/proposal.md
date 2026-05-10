# Proposal: Automatic Trusted-Signer Bootstrap on Bundle Apply

## Why

The host-signed-bundles change introduced `trusted_signers` in
`IntegrityConfig` and made `admin_manifest_bundle_handler` sign the
`integrity.lock` with the node's own Ed25519 key. However, operators still had
to manually call `GET /admin/identity/public-key`, copy the key, add it to
`trusted_signers` in their config, seal, and propagate — otherwise a node that
reboots would fail to reload the node-signed manifest (the embedded boot key
no longer matches).

The bootstrap step is purely mechanical: the only entity that knows which key
signed the manifest is the node itself, and it is signing it in the handler.
Injecting the key automatically is therefore safe and removes operator friction
with no change to the security model.

## What Changes

- `admin_manifest_bundle_handler` injects the node's public key into the
  `trusted_signers` list of the `IntegrityConfig` before signing, when the key
  is not already present. The injection is idempotent.
- The signed payload is the re-serialised JSON of the extended config
  (including `trusted_signers`), not the original client-supplied payload.
- The gossip checksum is computed from the final signed payload so peer nodes
  hash the same content.
- No operator action is required after the first bundle apply on a freshly
  provisioned node.
