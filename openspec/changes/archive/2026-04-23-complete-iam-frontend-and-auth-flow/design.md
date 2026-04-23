# Design: complete-iam-frontend-and-auth-flow

## Summary
This change replaces the legacy node connection overlay with a staged AuthN login and MFA flow, then promotes identity management into a dedicated routed dashboard inside the desktop shell.

## Frontend
The desktop shell now blocks navigation behind `#auth-overlay` until login and MFA complete.
The `Identity` view renders the active admin session, endpoint posture, and supported recovery actions.
The existing `My Account` PAT workflow remains separate to avoid mixing personal token issuance with shared IAM actions.

## Client And Tauri Bridge
The Tauri bridge exposes the minimum new command surface required by the frontend:
- `authn_login`
- `iam_list_users`
- `iam_regen_mfa`

The client stores the active authenticated connection profile, including the operator identity needed by the IAM view.
Remote IAM CRUD remains intentionally unsupported.

## Validation And Delivery
The change is validated through OpenSpec checks, frontend build, workspace lint/tests, CI, and a homelab rollout of `ghcr.io/astorise/tachyon-mesh:latest`.
