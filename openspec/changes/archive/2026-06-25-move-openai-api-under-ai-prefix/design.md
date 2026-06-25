## Context

The OpenAI-compatible guest is currently sealed at `/v1/models` and `/v1/chat/completions` and consumed through `https://ai.tachyon-mesh.wsl/v1`. This requires a dedicated hostname and certificate coverage. Tachyon already supports arbitrary sealed user-route paths, so the API can instead share the primary origin under an explicit product namespace.

The change is cross-cutting because route strings are part of guest dispatch, integrity manifests, import bundles, host tests, client calls, UI copy, IDE configuration, and the live HomeLab manifest.

## Goals / Non-Goals

**Goals:**

- Make `/ai/v1/models` and `/ai/v1/chat/completions` the only canonical public OpenAI-compatible routes.
- Use `https://tachyon-mesh.wsl/ai/v1` as the Continue and HomeLab API base.
- Preserve OpenAI request and response compatibility below the new base path.
- Keep the internal model-registration route unchanged.
- Make the migration explicit and test that `/v1/*` is no longer sealed.

**Non-Goals:**

- Adding redirects or compatibility aliases for `/v1/*`.
- Changing OpenAI payload schemas, authentication, model aliases, or inference behavior.
- Replacing ingress path mapping; external deployments may still map another origin or path to the canonical route.
- Moving `/internal/guest-openai/register` into the public `/ai` namespace.

## Decisions

1. **Use a path prefix on the primary origin.** The canonical API base is `https://tachyon-mesh.wsl/ai/v1`. This requires only the primary host certificate and keeps `/ai` available as the product namespace. A dedicated `ai.*` origin was rejected because certificate wildcard policy is deployment-dependent.

2. **Treat the move as breaking.** `/v1/models` and `/v1/chat/completions` are removed from manifests and dispatch constants. Redirects and aliases were rejected because they would preserve an ambiguous public contract and make route ownership harder to audit.

3. **Keep OpenAI version semantics intact.** The `/v1` portion remains immediately below `/ai`, so OpenAI-compatible clients continue to receive a conventional API base ending in `/v1`.

4. **Keep registration internal.** `/internal/guest-openai/register` is used by the model broker and is not a client-facing OpenAI endpoint, so moving it would mix internal integration with the public compatibility surface.

5. **Update both source and live sealed configuration.** Source manifests and sealing scripts prevent regressions in future builds; the HomeLab manifest is resealed and hot-reloaded so the running node matches the repository contract.

6. **Remove the dedicated HomeLab route.** `ai.tachyon-mesh.wsl` is deleted after the primary-origin smoke tests pass. Ingress users can independently map an external hostname to `/ai/v1/*`.

## Risks / Trade-offs

- **[Existing clients receive 404]** → Document the breaking base-URL change and update Continue in the same deployment.
- **[Stale sealed manifests retain `/v1/*`]** → Update sealing scripts, integrity fixtures, and validation tests; explicitly smoke-test old paths as 404.
- **[Internal comments and UI copy drift]** → Search the full active tree for path and hostname references, excluding archived historical changes where preserving history is intentional.
- **[Live manifest update interrupts inference]** → Use the existing signed bundle hot-reload path and verify readiness before and after the update.

## Migration Plan

1. Update route constants, manifests, clients, tests, documentation, and Continue configuration.
2. Build and test the guest and affected host/client/UI components.
3. Reseal the live manifest with `/ai/v1/*`, preserving model bindings and the internal registration route.
4. Smoke-test model listing and chat completions on `https://tachyon-mesh.wsl/ai/v1`.
5. Confirm `/v1/*` returns 404.
6. Remove `ai.tachyon-mesh.wsl` from HomeLab routing.

Rollback consists of restoring the previous sealed manifest and Continue base URL, then re-adding the dedicated HomeLab route if required.

## Open Questions

None.
