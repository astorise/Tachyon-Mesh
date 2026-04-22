# Design: Identity and Security Suite

## Recovery-Code Flow
- `system-faas-auth` owns recovery-code generation and redemption.
- Codes are generated as plaintext once, hashed before persistence, and stored in the auth state directory keyed by username.
- A consumed code is removed immediately and replaced with a short-lived JWT so the caller can recover access without a second factor.

## Host Surface
- `POST /admin/security/recovery-codes` is protected by admin auth and is used by onboarding.
- `POST /auth/recovery/consume` is intentionally unauthenticated so a locked-out operator can redeem a recovery code.

## UI Surface
- After the first successful node connection, the UI presents a security onboarding modal.
- The modal uses a QR placeholder step followed by a mandatory recovery-code step.
- Completion is blocked until the user downloads the recovery-code text bundle.
