## Context

The UI and MCP already share `tachyon-client` for remote node access and local workspace overlays. The implementation should avoid duplicating manifest mutation and signing logic across Tauri and MCP.

## Decisions

- Put overlay staging, sealing, Ed25519 signing, and manifest POST logic in `tachyon-client`.
- Keep `apply_configuration` validation in the Tauri app, but stage successfully validated payloads into the local overlay and return `staged/requiresSeal` state.
- Generate a local Ed25519 signing key under the workspace when no local key exists, then use it to sign the SHA-256 digest of the new config payload, matching the core-host verification path.
- Merge pending resources into `resources` and stage UI panel payloads under `ui_configurations`; core-host ignores unknown config fields while preserving the raw submitted manifest.
- Let the frontend observe `apply_configuration` responses centrally through `resilientInvoke`, so every panel can trigger the global Seal & Apply button without touching each panel.
- Initialize `tauri-plugin-stronghold` and route credential persistence through native commands so passwords are no longer written to browser `localStorage`.

## Trade-offs

- The first native credential commands establish the secure native boundary and remove browser cleartext persistence. A deeper follow-up can wire direct Stronghold vault item reads once the frontend chooses a stable vault password UX.
- Sealing writes `integrity.lock` before POSTing. If the host rejects the manifest, the local file still reflects the attempted signed payload for inspection and retry.
