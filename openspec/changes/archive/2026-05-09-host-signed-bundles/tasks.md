# Tasks

- [x] 1. Add `trusted_signers: Vec<String>` (serde default empty) to
       `IntegrityConfig` in `core-host/src/host_core/integrity_config.rs`.
- [x] 2. Update `verify_integrity_payload` / `verify_integrity_signature` to
       accept any key present in the active config's `trusted_signers` list in
       addition to the key embedded in the manifest itself, guarded by the
       `AppState` context passed in.
- [x] 3. Update `admin_manifest_bundle_handler` to sign the `configPayload`
       with `state.host_identity.signing_key` instead of delegating back to
       the client; stop requiring `signature`/`publicKey` fields in the
       incoming YAML.
- [x] 4. Update `build_deployment_bundle` in `tachyon-client` to omit the
       `publicKey` and `signature` lines from `manifest.yaml`.
- [x] 5. Add `GET /admin/identity/public-key` returning the node's current
       hex-encoded Ed25519 public key so Studio can display it and operators
       can add it to peer `trusted_signers`.
- [x] 6. Validate with `openspec validate --all` and commit.
