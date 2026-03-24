## 1. Signer CLI

- [x] 1.1 Create a `cli-signer` binary crate in the workspace and add the dependencies required for Ed25519 signing, hashing, JSON serialization, and hex encoding.
- [x] 1.2 Implement `cli-signer` so it generates a signing key, hashes the canonical configuration payload, signs the hash, and writes `integrity.lock` at the workspace root with `config_payload`, `public_key`, and `signature`.

## 2. Build Integration

- [x] 2.1 Add `core-host/build.rs` and the required build dependencies so the host build reads `../integrity.lock`.
- [x] 2.2 Make the build script emit `cargo:rerun-if-changed=../integrity.lock` and expose `FAAS_CONFIG`, `FAAS_PUBKEY`, and `FAAS_SIGNATURE` as compile-time environment variables.

## 3. Runtime Verification

- [x] 3.1 Add the runtime dependencies needed in `core-host` for Ed25519 verification, hashing, and hex decoding.
- [x] 3.2 Verify the sealed configuration at the start of `core-host/src/main.rs` and abort startup immediately if signature validation fails.

## 4. Validation

- [x] 4.1 Run `cargo run -p cli-signer` to generate `integrity.lock`.
- [x] 4.2 Run `cargo run -p core-host` to confirm the host embeds the manifest values during build.
- [x] 4.3 Confirm the host logs that integrity verification passed for a valid manifest and fails fast after any manifest tampering.
