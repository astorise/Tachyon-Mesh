# Tasks: advanced-iam-and-identity-decoupling

- [x] Normalize the change artifacts to the current OpenSpec delta layout for identity and desktop UI capabilities.
- [x] Rename `systems/system-faas-auth` to `systems/system-faas-authn`, add `systems/system-faas-authz`, and wire both crates into the workspace and Docker packaging.
- [x] Split the identity WIT surface into dedicated AuthN and AuthZ worlds and implement JWT plus PAT validation in AuthN together with scope-based policy evaluation in AuthZ.
- [x] Update `core-host` so every `/admin/*` request is authenticated through AuthN and then authorized through AuthZ in both HTTP/1.1 and HTTP/3 paths.
- [x] Add PAT issuance endpoints, shared client helpers, Tauri commands, and a `My Account` view in `tachyon-ui`.
- [x] Validate the change with `cargo check --workspace`, targeted tests, and `openspec validate --changes`.
