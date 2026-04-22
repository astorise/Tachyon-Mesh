# Proposal: advanced-iam-and-identity-decoupling

## Why
The current administrative security plane still routes every decision through a single `system-faas-auth` component. That couples credential verification, recovery-code flows, and authorization policy in one runtime path, and it does not support scoped Personal Access Tokens for CI/CD automation. The desktop UI also lacks a dedicated personal security surface where operators can mint PATs and rotate their own recovery posture.

## What Changes
- Split the current identity component into `system-faas-authn` and `system-faas-authz` with dedicated WIT contracts.
- Add PAT issuance and validation in AuthN, with hashed persistence in the existing auth state store.
- Change host-side admin authorization so `/admin/*` requests run through a two-step AuthN then AuthZ pipeline using the request method and path.
- Extend `tachyon-ui` with a dedicated `My Account` view for PAT issuance and personal security actions.

## Impact
- Workspace membership, Docker packaging, and module resolution must be updated for the renamed AuthN component and the new AuthZ component.
- Administrative bearer tokens can now be either JWTs or scoped PATs.
- The desktop client and Tauri wrapper gain a PAT issuance path without changing the existing connection bootstrap flow.
