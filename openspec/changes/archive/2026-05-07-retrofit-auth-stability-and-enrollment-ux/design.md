## Context

The recent fixes crossed the host router, AuthN state storage, Tauri client commands, the desktop login/enrollment overlay, and the `<tachyon-iam>` web component. Before the fix, a missing `/auth/*` route could be reported as an `integrity.lock` sealed-route failure, Home Lab AuthN state lived inside the pod filesystem, and enrollment depended on the login step for the node URL.

## Goals / Non-Goals

**Goals:**
- Keep AuthN/Admin routes owned by `core-host` and independent from sealed FaaS route integrity.
- Preserve Home Lab account state across pod replacement and image upgrades.
- Make password login a staged flow that requires MFA finalization before unlocking the UI.
- Make enrollment self-contained, including node URL input, invite validation, password confirmation, QR-backed TOTP, and final MFA verification.
- Provide consistent behavior between the desktop overlay and the reusable IAM web component.

**Non-Goals:**
- Redesign AuthN token formats, password hashing, or TOTP algorithms.
- Make saved credentials a cross-device sync feature.
- Allow desktop UI resource changes to bypass `integrity.lock` re-sealing.
- Move non-system FaaS routes out of sealed route validation.

## Decisions

- Reserve `/auth/*` and `/admin/*` in the host router before sealed route lookup.
  - Rationale: System recovery and configuration must remain reachable even when the FaaS integrity manifest is stale or compromised.
  - Alternative considered: Add AuthN/Admin routes to `integrity.lock`; rejected because it preserves the lockfile dependency for recovery paths.

- Persist Home Lab AuthN state with a Kubernetes PVC mounted at `/app/auth-state`.
  - Rationale: The AuthN component already writes durable state under this path; the missing piece was deployment persistence.
  - Alternative considered: Bake initial accounts into the image; rejected because account material must remain runtime state.

- Treat password authentication as stage/finalize rather than direct bearer-token connection.
  - Rationale: Password acceptance alone must not unlock the UI; the final step is the 6-digit MFA code.
  - Alternative considered: Reuse the remote admin status endpoint with password material; rejected because it conflates credentials with admin bearer tokens.

- Keep enrollment node URL input synchronized with the login node URL.
  - Rationale: Operators can start enrollment directly while still preserving the familiar login URL field.
  - Alternative considered: Force all enrollment through the login step; rejected because it caused first-run confusion and unnecessary backtracking.

- Store remembered credentials only after the operator opts in.
  - Rationale: This is a workstation convenience with explicit consent, and MFA remains required for the final login step.
  - Alternative considered: Persist only username and URL; rejected for the requested workstation-save behavior, but password persistence remains opt-in.

## Risks / Trade-offs

- Saved password material in local storage increases workstation risk if the local profile is compromised. Mitigation: require explicit opt-in and keep MFA as the final unlock step.
- Reserved route prefixes reduce namespace available for guest FaaS paths. Mitigation: system prefixes are intentionally owned by core-host and should not be used for guest routes.
- PVC-backed Home Lab state preserves accounts but also preserves bad local state. Mitigation: recovery token and admin recovery workflows remain available for controlled reset.
- Web component and desktop overlay can drift. Mitigation: both surfaces carry the same required controls and invocation behavior in the specs.

## Migration Plan

- Deploy the core-host image containing reserved route handling and staged login/finalize support.
- Apply the Home Lab manifest with the `tachyon-auth-state` PVC mounted at `/app/auth-state`.
- Reinstall Tachyon-UI so the local desktop overlay and web component expose the new login/enrollment controls.
- Re-enroll the first administrator if the previous pod-local state was lost before the PVC was added.
