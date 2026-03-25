## 1. Tauri Workspace Setup

- [x] 1.1 Add a `tachyon-cli` workspace member configured as a Tauri application.
- [x] 1.2 Configure the Tauri CLI feature so the app accepts a `generate` subcommand with repeatable `--route` input and a `--memory` limit option.

## 2. Manifest Generation Backend

- [x] 2.1 Implement backend logic that assembles the configuration payload, generates an Ed25519 key pair, hashes and signs the payload, and writes a compatible `integrity.lock`.
- [x] 2.2 Wire the `generate` CLI path so it runs headlessly and exits without opening a desktop window.

## 3. Migration and Verification

- [x] 3.1 Update workspace references so `tachyon-cli` becomes the supported manifest-generation tool.
- [x] 3.2 Remove `cli-signer` once `tachyon-cli` can fully replace its output contract.
- [x] 3.3 Verify that `core-host` still builds and validates manifests produced by `tachyon-cli`.
