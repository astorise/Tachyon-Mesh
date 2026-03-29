# Tasks: Change 019 Implementation

## 1. OpenSpec and Manifest Model

- [x] 1.1 Rewrite the proposal, design, and delta specs so `secrets-management` matches the implemented manifest grant model (`allowed_secrets`) and the `secrets-vault` WIT import.
- [x] 1.2 Extend the signed manifest schema and `tachyon-cli generate` workflow to seal named secret grants through `--secret-route`.

## 2. Runtime and Guest Integration

- [x] 2.1 Add the `secrets-vault` feature flag to `core-host`, load a mock in-memory vault when enabled, and keep secrets out of the WASI environment block.
- [x] 2.2 Implement the host-side `secrets-vault` WIT binding with `vault-disabled`, `permission-denied`, and successful lookup behavior based on sealed route grants.
- [x] 2.3 Update `guest-example` to prove that `DB_PASS` is absent from `std::env` but available through the typed vault import when granted.

## 3. Verification

- [x] 3.1 Rebuild the guest artifacts and verify `core-host` without and with `--features secrets-vault`.
- [x] 3.2 Run workspace-level validation (`cargo test --workspace`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `openspec validate --all`).
- [x] 3.3 Archive `secrets-management` with spec sync once the branch head is green.
