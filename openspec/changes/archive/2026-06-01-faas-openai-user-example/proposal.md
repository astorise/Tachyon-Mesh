## Why

The OpenAI-compatible layer (`system-faas-openai-adapter` + `system-faas-ai-list-model`) was modelled as two **system** FaaS that are runtime-injected under `#[cfg(feature = "ai-inference")]`. This made them invisible in Topology unless a node was compiled with `ai-inference`, left them with no dependency edges (injected routes carry empty `dependencies`), and forced an artificial mesh hop between the adapter and the registry. Functionally these components are not control-plane infrastructure — they are an **application-level demonstration** of building an OpenAI-compatible API on Tachyon. They belong in user space, as an example.

## What Changes

- **New user FaaS example `guest-openai`** (`examples/guest-openai`, WIT world `faas-guest`) that absorbs both halves of the OpenAI layer in a single module:
  - OpenAI read endpoints: `GET /v1/models`, `POST /v1/chat/completions` (stub `501`).
  - Registry endpoints previously served by `ai-list-model`: register, list-models, deregister.
  - Backed **directly** by the `kv-partition` table `ai-models-registry` (the `faas-guest` world already imports `kv-partition`), so **no `outbound-http` and no WIT change** are required.
- **Cache removed** — the registry list is read fresh from `kv-partition` on every request (the old `thread_local` `MODELS_CACHE` is dropped), guaranteeing a just-uploaded model is visible immediately, including across multiple guest instances.
- **Scope grant** — the `guest-openai` route declares `scopes.kv` granting the `ai-models-registry` table; this scoped grant is the access boundary (table open is gated by deployment scopes, not by role).
- **`model-broker` stays system** and keeps notifying new uploads over HTTP (it runs in `system-faas-guest`, which has no `kv-partition`); its register URL targets `guest-openai`.
- **BREAKING — system decommission**: `system-faas-openai-adapter` and `system-faas-ai-list-model` crates are removed, along with their `systems/manifest.toml` entries; `inject_feature_routes` no longer injects `/system/ai-openai-adapter` or `/system/ai-list-model` (only `/system/model-broker` remains under `ai-inference`).
- **Example package wiring** — the OpenAI surface is declared as user routes in `examples/guest-examples/manifest.json` (`openai-models`, `openai-chat`, `openai-registry`), each with `targets.module = guest-openai`, so Topology renders endpoint→custom-wasm pairs all backed by the `guest-openai` module without relying on runtime injection. (Routes must carry distinct names — the validator rejects same-name/same-version routes — so the layer appears as sibling nodes sharing one asset source rather than a single node.) The former adapter→registry edge becomes an intra-module relationship (the two are merged), so no synthetic cross-route edge is fabricated.

## Capabilities

### New Capabilities

- `openai-compatible-faas`: A user-space FaaS that exposes the OpenAI-compatible HTTP surface and owns the model registry through `kv-partition`, with fresh-read semantics, scope-gated table access, and HTTP-based upload notification from `model-broker`.

### Modified Capabilities

- `ai-orchestration`: The OpenAI adapter and AI model registry requirements move from the `system-faas-openai-adapter` / `system-faas-ai-list-model` system FaaS to the `guest-openai` user FaaS; model listing reads the registry directly via `kv-partition` rather than calling a separate registry FaaS; the gateway no longer dispatches `/v1/*` to a system adapter (the routes resolve directly to the user FaaS). The `model-broker` predictive-prewarm and VRAM requirements are unchanged.
- `feature-auto-injection`: the `ai-inference` injection bundle is reduced to `/system/model-broker` only; `/system/ai-list-model` and `/system/ai-openai-adapter` are no longer injected (supersedes the prior `feature-auto-injection` / `topology-drag-and-system-routes` behavior).
- `faas-package-import`: the guest-examples manifest additionally declares the `guest-openai` OpenAI routes, so an import activates 12 routes (9 guest + 3 OpenAI).

## Impact

- **New**: `examples/guest-openai/` (crate + `manifest`-declared routes).
- **Removed**: `systems/system-faas-openai-adapter/`, `systems/system-faas-ai-list-model/`, their two `systems/manifest.toml` entries.
- **`core-host/src/host_core/integrity_config.rs`**: `inject_feature_routes` AI bundle reduced to `model-broker`.
- **`systems/system-faas-model-broker/src/lib.rs`**: register-notification URL retargeted to `guest-openai`.
- **`examples/guest-examples/manifest.json`**: add `guest-openai` user routes with module target + dependencies.
- **`tachyon-client/src/lib.rs`**: `get_cluster_features` drops the stale `ai-list-model` slug term from `has_ai` (still satisfied by `model-broker`).
- **Topology**: the OpenAI surface now appears as user endpoint→custom-wasm pairs (`openai-models` / `openai-chat` / `openai-registry`, all sourced from the `guest-openai` module), present regardless of `ai-inference` — the original reporting gap (invisible adapter, no edges) is closed: the adapter↔registry link is now internal to the merged module.
