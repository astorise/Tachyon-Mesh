## Why

The OpenAI-compatible API currently depends on a dedicated `ai.*` hostname, which makes local and downstream deployments depend on wildcard or explicit subdomain certificate management. Publishing the API under the primary Tachyon origin with an `/ai/v1/*` prefix provides a stable contract that works with a single host certificate and can still be remapped by an ingress.

## What Changes

- **BREAKING** Move the public model-list endpoint from `/v1/models` to `/ai/v1/models`.
- **BREAKING** Move the public chat-completions endpoint from `/v1/chat/completions` to `/ai/v1/chat/completions`.
- Remove the former public `/v1/*` routes rather than retaining compatibility aliases.
- Make `https://tachyon-mesh.wsl/ai/v1` the canonical HomeLab and Continue API base.
- Remove the dedicated `ai.tachyon-mesh.wsl` HomeLab route.
- Update sealed manifests, examples, clients, tests, UI copy, MCP descriptions, and documentation to use the prefixed paths.
- Keep `/internal/guest-openai/register` unchanged because it is an internal integration route rather than part of the OpenAI-compatible public surface.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `openai-compatible-faas`: Change the canonical public OpenAI-compatible paths to `/ai/v1/*` and require the former `/v1/*` paths to be absent.
- `ai-orchestration`: Change model discovery references to the prefixed model-list endpoint.
- `faas-package-import`: Seal and import the OpenAI-compatible example routes under `/ai/v1/*`.
- `ai-model-upload-ui`: Report the new model-list path after upload.

## Impact

Affected areas include the `guest-openai` route dispatcher, integrity manifests and sealing scripts, gateway pass-through tests, Tachyon client and MCP messaging, UI text and tests, OpenSpec requirements, Continue configuration, HomeLab HTTP routing, and deployment smoke tests. Existing clients configured with `/v1` must update their base URL to `/ai/v1`.
