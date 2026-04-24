# Implementation Tasks

## Phase 1: Auth contract and backend
- [x] Extend `wit/authn.wit` with `validate-registration-token`, `stage-user`, and `finalize-enrollment`.
- [x] Implement invite-token validation, staged enrollment persistence, TOTP verification, and one-time token consumption in `system-faas-authn`.
- [x] Add regression coverage for invite signup and token consumption in the AuthN test suite.

## Phase 2: Host and client wiring
- [x] Expose public signup endpoints in `core-host` outside the protected `/admin/*` middleware.
- [x] Update `tachyon-client` to call the new signup endpoints and persist the returned authenticated session.
- [x] Surface the new client calls through Tauri commands in `tachyon-ui/src/main.rs`.

## Phase 3: Desktop UI flow
- [x] Add a `Register with Invite Token` path to the existing auth overlay in `tachyon-ui/index.html`.
- [x] Implement the staged signup wizard in `tachyon-ui/src/main.ts` using the repo's current HTML/TypeScript architecture.
- [x] Render the provisioning URI as a QR code and require the first 6-digit TOTP code before unlocking the dashboard.

## Phase 4: Validation and polish
- [x] Preserve auto-login after successful enrollment and route the user into the existing dashboard shell.
- [x] Add user-facing error handling for invalid, expired, or reused signup state.
- [x] Update the first-run recovery-code onboarding flow to target the active operator instead of a hard-coded username.
