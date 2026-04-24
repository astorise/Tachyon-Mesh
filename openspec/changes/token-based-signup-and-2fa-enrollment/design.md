# Design: Token-based Signup & Mandatory 2FA Enrollment

## 1. Backend contract and persistence
The existing `tachyon:identity/authn` WIT contract is extended instead of creating a new service surface:
- `validate-registration-token(token)` validates invite JWTs with `token_use=registration`
- `stage-user(token, profile)` persists a pending enrollment session and returns an `otpauth://` URI
- `finalize-enrollment(session-id, totp-code)` activates the user, burns the invite token, and returns an authenticated session

The AuthN component persists:
- user security records under the existing auth-state root
- pending signup sessions in a dedicated `pending-enrollments/` subdirectory
- consumed invite-token hashes in a dedicated `registration-tokens/` subdirectory

This avoids polluting the PAT/user-state files already scanned by the current host runtime.

## 2. Host and client integration
`core-host` exposes three unauthenticated signup endpoints outside `/admin/*`:
- `POST /auth/signup/validate-token`
- `POST /auth/signup/stage`
- `POST /auth/signup/finalize`

`tachyon-client` and the Tauri command layer reuse those endpoints, then store the returned JWT in the existing authenticated connection profile so the enrolled operator lands directly in the desktop dashboard.

## 3. Desktop UI implementation
The current repository does not use React or Zustand for `tachyon-ui`; it uses a static HTML shell with a Tauri/TypeScript controller (`index.html` + `src/main.ts`).

The signup flow is therefore implemented as additional staged panels inside the existing auth overlay:
- login step with secondary action `Register with Invite Token`
- invite validation step
- profile staging step
- TOTP enrollment step with QR rendering through the `qrcode` package

GSAP is reused for panel transitions so the overlay remains consistent with the rest of the desktop shell.

## 4. Security considerations
- No account is activated before the first valid TOTP code.
- Invite tokens are capped to 24 hours and rejected once consumed.
- Expired signup sessions are pruned before new staging attempts.
- Recovery-code onboarding remains mandatory after the first authenticated desktop session.
