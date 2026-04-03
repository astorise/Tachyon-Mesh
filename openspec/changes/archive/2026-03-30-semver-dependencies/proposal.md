# Proposal: Change 025 - SemVer Dependency Resolution & IPC Routing

## Context
As the FaaS ecosystem grows, functions will depend on other functions via the internal Mesh (IPC). Hardcoding exact target versions or relying purely on A/B traffic splits is fragile. FaaS A (v2.0.0) might strictly require FaaS B (>=3.1.0) because of a change in the JSON payload structure. Tachyon Mesh needs to act as a runtime dependency resolver.

## Objective
Introduce Semantic Versioning (SemVer) to the `integrity.lock` manifest. Each FaaS target will declare its own version and a map of its outbound dependencies with SemVer constraints. The `core-host` will validate this dependency graph at boot to prevent runtime crashes, and dynamically route inter-FaaS calls to the correct matching version.

## Scope
- Add the `semver` crate to both `tachyon-cli` and `core-host`.
- Update the `Target` schema in the config to include a `version` string and a `dependencies` map.
- Implement a **Graph Validator** in the `core-host` startup sequence that ensures all declared constraints can be satisfied by the loaded modules.
- Update the internal IPC router (Change 010 / 024): when FaaS A makes a mesh call to `faas-b`, the Host looks up FaaS A's dependency constraints and routes the call to the highest loaded version of `faas-b` that satisfies it.

## Success Metrics
- If the `integrity.lock` specifies FaaS A depends on `faas-b: "^3.1.0"`, but only `faas-b: "3.0.0"` is provided in the manifest, the `core-host` panics at startup with a clear dependency error.
- If valid, FaaS A's outbound call to `http://mesh/faas-b` is automatically routed by the Host to the `3.1.5` instance, even if a `4.0.0` instance exists for other clients.