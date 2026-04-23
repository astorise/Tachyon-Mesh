# Proposal: complete-iam-frontend-and-auth-flow

## Why
The desktop shell already contains the underlying admin connection, recovery-code, and PAT capabilities, but the UI still exposes a legacy one-step connection overlay and an underpowered identity panel. The OpenSpec change itself is also malformed, so it cannot be validated, applied, or archived. We need to normalize the change and finish the frontend auth gateway and IAM surface in a way that matches the backend that actually exists today.

## What Changes
- Normalize the change into valid OpenSpec proposal, task, and delta-spec artifacts.
- Replace the legacy `connection-overlay` with a full-screen AuthN gateway that stages login and MFA before unlocking the dashboard.
- Expand the Identity view into a routed IAM dashboard that renders the active administrative subject, endpoint posture, recovery-bundle status, and direct security actions backed by the existing Tauri/client layer.

## Impact
- `tachyon-ui/index.html` and `tachyon-ui/src/main.ts` become the canonical source for the two-step auth gateway and IAM dashboard behavior.
- `tachyon-ui/src/main.rs` and `tachyon-client/src/lib.rs` expose a small, explicit auth/IAM command surface for the frontend without inventing new remote cluster APIs.
- The change becomes valid for `openspec validate`, ready for CI, deployable to the homelab cluster, and archivable.
