## Why

The OpenAI-compatible API advertises uploaded models from `GET /v1/models`, but
`POST /v1/chat/completions` rejects the same aliases when they are absent from
that route's sealed model bindings. This makes standards-compatible clients
such as Continue discover a model they cannot use.

## What Changes

- Allow the OpenAI chat route to use broker-uploaded model aliases that are
  registered and present under the managed dynamic-model directory.
- Move the canonical OpenAI-compatible surface to the dedicated HTTPS origin
  `https://ai.tachyon-mesh.wsl`, retaining the standard `/v1/models` and
  `/v1/chat/completions` paths only on that origin.
- Remove the OpenAI surface from `https://tachyon-mesh.wsl`.
- Add regression coverage proving that a listed dynamic model can be selected
  by the chat route without granting unrelated routes access to it.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `openai-compatible-faas`: A registered dynamic model listed by `/v1/models`
  must be usable by `/v1/chat/completions` on the same deployment.
- `ai-inference`: Dynamic model authorization must remain route-scoped while
  supporting broker-registered aliases.

## Impact

- OpenAI guest route model authorization and integrity configuration.
- Deployment manifest generation/sealing for `guest-openai`.
- Continue's local provider endpoint.
- HomeLab DNS and HTTPS routing for `ai.tachyon-mesh.wsl`.
- Existing clients must update their OpenAI base URL to the new origin.
