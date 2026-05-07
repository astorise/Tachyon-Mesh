## Why

Recent authentication regressions showed two specification gaps: critical AuthN/Admin routes could appear coupled to sealed FaaS routing, and Home Lab account state was not guaranteed to survive a deployment rollout. The desktop enrollment flow also forced operators to leave enrollment to enter the node URL, making first-run recovery unnecessarily fragile.

## What Changes

- Document that `/auth/*` and `/admin/*` are reserved core-host system routes and must not fall through to `integrity.lock` sealed FaaS routing.
- Specify password-based login staging plus final MFA verification for the desktop AuthN flow.
- Specify QR-backed TOTP enrollment behavior and explicit rejection of invalid enrollment state.
- Add desktop UX requirements for password visibility toggles, optional local credential persistence, and a Mesh Node URL field inside enrollment.
- Add Home Lab deployment requirements for persistent AuthN state mounted at `/app/auth-state`.

## Capabilities

### New Capabilities
- `homelab-auth-state-persistence`: Home Lab deployment requirements for preserving AuthN account state across image upgrades and pod replacement.

### Modified Capabilities
- `http-routing`: Reserve system auth/admin route prefixes in core-host before sealed FaaS route lookup.
- `identity-and-security-suite`: Stabilize login/enrollment requirements for staged password login, MFA finalization, QR-backed TOTP, and local credential UX.
- `iam-webcomponent`: Mirror the stabilized AuthN and enrollment controls in the `<tachyon-iam>` web component.

## Impact

- Affects `core-host` HTTP routing behavior for unknown `/auth/*` and `/admin/*` paths.
- Affects AuthN and enrollment commands used by `tachyon-ui`.
- Affects desktop UI login/enrollment screens and the `tachyon-iam` web component.
- Affects `manifests/homelab.yaml` by requiring a persistent volume claim for AuthN state.
