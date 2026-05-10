# Tasks

- [x] 1. In `admin_manifest_bundle_handler`, after parsing the incoming
       `configPayload` into `IntegrityConfig`, push the node's
       `host_identity.public_key_hex` into `config.trusted_signers` when not
       already present.
- [x] 2. Re-serialize the extended config to JSON before signing so the
       Ed25519 signature covers the `trusted_signers` field.
- [x] 3. Use the re-serialised payload for the gossip checksum and the
       written `integrity.lock` so all three are consistent.
- [x] 4. Validate that `cargo check` passes without errors and that existing
       IAM + audit tests remain green.
- [x] 5. Archive this change and commit.
