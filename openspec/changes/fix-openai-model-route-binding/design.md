## Context

Tachyon currently exposes the local cluster through
`https://tachyon-mesh.wsl`. The OpenAI guest owns `/v1/models` and
`/v1/chat/completions`. Uploaded models are stored dynamically and registered in
the shared model table, but the component host authorizes accelerator access
only from the route's statically materialized `models` collection.

The current normalizer also discards the `dynamic` flag and requires a static
path, preventing the integrity manifest from expressing a route-scoped dynamic
alias cleanly.

## Goals / Non-Goals

**Goals:**

- Make a registered uploaded model usable by the OpenAI chat route.
- Preserve route-scoped model authorization.
- Use the dedicated HTTPS hostname `ai.tachyon-mesh.wsl` from Continue.
- Keep the standard OpenAI `/v1` path layout.

**Non-Goals:**

- Grant every route access to every uploaded model.
- Keep the OpenAI routes on `tachyon-mesh.wsl`.
- Change authentication semantics or model execution support.

## Decisions

1. Preserve `IntegrityModelBinding.dynamic` during normalization. Dynamic
   bindings may omit a static path because the runtime resolves them under the
   managed broker model directory.
2. Declare uploaded aliases as dynamic bindings on the
   `/v1/chat/completions` route. This keeps authorization explicit and local to
   the route.
3. Keep `/v1/*` canonical under the dedicated `ai.tachyon-mesh.wsl` origin.
   Host-based routing provides isolation without adding a nonstandard path
   prefix or retaining duplicate endpoints.
4. Configure Continue with `https://ai.tachyon-mesh.wsl/v1` and the local CA
   behavior required by the workstation.

## Risks / Trade-offs

- [A registry entry can exist without a matching dynamic binding] → Keep the
  binding explicit in the sealed manifest and add a regression test.
- [The uploaded model format may still be unsupported at execution time] →
  Surface the typed runtime error separately from authorization failures.
- [Local clients may not trust the Home Lab CA] → Configure Continue's request
  verification appropriately for this workstation; do not weaken the Tachyon
  server endpoint.

## Migration Plan

1. Update normalization and tests.
2. Add the dynamic binding to the deployed OpenAI chat route and reseal/reload
   the manifest.
3. Configure HomeLab DNS and HTTPS routing for `ai.tachyon-mesh.wsl`, removing
   the previous `tachyon-mesh.wsl` route.
4. Verify `/v1/models` and `/v1/chat/completions` through HTTPS.
5. Point Continue at the HTTPS origin and remove the obsolete port-forward.

Rollback restores the prior manifest and Continue configuration.

## Open Questions

None.
