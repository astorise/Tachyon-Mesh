## 1. Core AuthN Routing

- [x] 1.1 Register `/auth/login/stage` and `/auth/login/finalize` as core-host system routes.
- [x] 1.2 Prevent unknown `/auth/*` and `/admin/*` paths from falling through to sealed FaaS route lookup.
- [x] 1.3 Verify unknown system routes return a system-route 404 instead of an `integrity.lock` sealed-route error.

## 2. AuthN State Persistence

- [x] 2.1 Add Home Lab persistent volume claim for AuthN state.
- [x] 2.2 Mount the persistent volume at `/app/auth-state` in the `tachyon-host` deployment.
- [x] 2.3 Roll out the Home Lab deployment with the persistent AuthN state mount.

## 3. Desktop Login and MFA

- [x] 3.1 Change desktop password login to use staged login instead of treating the password as an admin bearer token.
- [x] 3.2 Store the MFA session returned by login staging and unlock only after finalization succeeds.
- [x] 3.3 Add password visibility and explicit remember-credentials controls to the desktop login flow.
- [x] 3.4 Preserve MFA as the final step when remembered credentials are restored.

## 4. Enrollment UX

- [x] 4.1 Render enrollment TOTP provisioning URI as a QR code and manual secret.
- [x] 4.2 Add password visibility controls to enrollment password fields.
- [x] 4.3 Add Mesh Node URL input to the invite-token enrollment step.
- [x] 4.4 Synchronize login and enrollment node URL fields.
- [x] 4.5 Reject enrollment invite validation, staging, or finalization when the node URL is missing.

## 5. IAM Web Component

- [x] 5.1 Mirror staged login, MFA finalization, remember-credentials, and password visibility controls in `<tachyon-iam>`.
- [x] 5.2 Add enrollment node URL input and URL synchronization to `<tachyon-iam>`.
- [x] 5.3 Render enrollment QR code in `<tachyon-iam>` and emit handled errors on QR generation failure.

## 6. Verification and Release

- [x] 6.1 Run frontend production build.
- [x] 6.2 Run Tauri desktop build.
- [x] 6.3 Reinstall Tachyon-UI on the Windows workstation.
- [x] 6.4 Commit and push the implementation commits.
