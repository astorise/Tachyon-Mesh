# Design: Complete Auth Flow

## Context

The UI already has staged login, enrollment, QR generation, and a Tauri Stronghold plugin setup, but credential persistence is stubbed and write operations can run without a fresh MFA proof.

## Decisions

- Keep browser `localStorage` limited to non-credential UI preferences and tour state.
- Use native Tauri commands as the secure boundary for remembered auth profile data and custom CA material. The Stronghold plugin remains initialized at startup, and the frontend no longer writes credentials to browser storage.
- Apply step-up authentication centrally in `resilientInvoke` for write commands so all panels inherit the same 20-minute sudo grace period.
- Add an admin-mode rendering path to `<tachyon-iam>` so the authenticated shell can reuse the IAM component for invite generation.
- Render invite QR codes with the existing `qrcode` dependency.

## Risks

- The current core enrollment endpoint returns a session id and PIN rather than a full operator signup token. The UI labels the PIN as the manual invite token and encodes the session in the QR payload.
- Native TOTP verification is currently format-gated at the Tauri boundary because the backend does not expose a dedicated session step-up verification endpoint separate from login finalization.
