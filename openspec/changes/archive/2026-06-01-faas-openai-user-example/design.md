## Context

Today the OpenAI layer is two system FaaS:

- `system-faas-ai-list-model` (world `control-plane-faas`) — persistent registry over the `ai-models-registry` `kv-partition` table; serves `/internal/ai-list-model/{register,list-models,deregister}`; caches the list in a `thread_local`.
- `system-faas-openai-adapter` (world `system-faas-guest`) — calls the registry over `outbound-http` (`http://mesh/internal/ai-list-model/list-models`) and reshapes the result to OpenAI JSON.

Both are runtime-injected by `inject_feature_routes` under `#[cfg(feature = "ai-inference")]` with empty `dependencies`, so they are invisible in Topology without `ai-inference` and never produce dependency edges. `system-faas-model-broker` (world `system-faas-guest`, no `kv-partition`) notifies the registry of new uploads via a best-effort HTTP POST.

Two host facts drive the design:

- `kv-partition` table keys are **global by table name** (`kv_partition::<name>`, no per-guest namespacing) and table open is **gated by deployment scopes** (`scopes.check_kv`), not by guest role.
- The `faas-guest` (user) world **imports `kv-partition`** but **not `outbound-http`**; raw external egress is system-only.

## Goals / Non-Goals

**Goals:**
- Collapse the adapter + registry into one user FaaS (`guest-openai`) that reads/writes `ai-models-registry` directly.
- Make a just-uploaded model immediately listable, including across instances.
- Render the OpenAI layer in Topology (node + dependency edge) from the example manifest, independent of `ai-inference`.
- No WIT change.

**Non-Goals:**
- Implementing real `/v1/chat/completions` inference (stays a `501` stub).
- Making `model-broker`'s upload notification reliable/transactional (it stays best-effort HTTP; retry is a separate change).
- Changing `model-broker`'s VRAM / predictive-prewarm behavior.

## Decisions

**D1 — One merged user FaaS, not two.** The adapter and registry are folded into `guest-openai`. Rationale: the only reason they were split was the system/control-plane capability boundary; once in user space the adapter can read `kv-partition` directly, eliminating the mesh hop and the need for `outbound-http` in the user world. Alternative (keep them separate, add `outbound-http` to `faas-guest`) was rejected: it broadens a capability for all user FaaS purely to preserve an internal hop.

**D2 — Read-through, no cache.** Drop the `thread_local` `MODELS_CACHE`. A `get_range` over a small registry table backed by local ReDB is cheap, and the cache caused stale reads across instances (register invalidates only the local instance). Alternative (shared invalidation / TTL) adds coordination for no measurable win at registry sizes.

**D3 — `model-broker` keeps the HTTP notify.** It runs in `system-faas-guest` with no `kv-partition`, so it cannot write the table directly. It POSTs to `guest-openai`'s register route; `guest-openai` performs the write. The mesh call is allowed because `model-broker` is system and the target is a sealed route. We retarget the hard-coded URL constant from `…/internal/ai-list-model/register` to the `guest-openai` register path.

**D4 — Scope-gated table access is the security boundary.** `guest-openai`'s route declares `scopes.kv` for `ai-models-registry`. Because table access is scope- not role-gated, moving the registry to user space does not weaken isolation: only routes the operator seals with that grant can touch the table.

**D5 — Topology comes for free.** Declaring the OpenAI surface as user routes (`/v1/models`, `/v1/chat/completions`, register), each with `targets.module = guest-openai`, makes them endpoint→custom-wasm pairs. They must have **distinct route names** (`openai-models`, `openai-chat`, `openai-registry`) because `validate_integrity_config` rejects same-name/same-version routes; the topology therefore shows sibling custom-wasm nodes that share the `guest-openai` asset source rather than one merged node. No topology code changes. We do not fabricate a synthetic dependency edge: the former adapter→registry hop is now intra-module (D1), so the honest graph has no cross-route edge there. The original gap (invisible adapter) closes because the routes are sealed user routes, not feature-injected.

## Risks / Trade-offs

- **[Multi-instance write/read consistency]** → ReDB transactions are atomic and the table is global-by-name, so a committed register is visible to every instance's next read once the cache is gone (D2).
- **[Best-effort upload notification]** → A dropped `model-broker` POST means the model is not listed until re-notified; this matches today's behavior and is called out as a Non-Goal. Mitigation deferred to a retry change.
- **[Global table namespace]** → Any sealed route granted `scopes.kv` for `ai-models-registry` can read/write it. Mitigated by operator-controlled scopes in the signed manifest (D4); document that the grant should be limited to `guest-openai`.
- **[`has_ai` feature gating]** → `has_ai` previously OR'd `ai-list-model`; with that slug decommissioned it still resolves via `model-broker` (kept system). We drop the stale term to avoid referencing a removed slug.
- **[`ai-inference` coupling]** → `model-broker` stays injected under `ai-inference`; `guest-openai` is always present in the example. With no `ai-inference`, `model-broker` is absent and the registry simply stays empty — `guest-openai` returns an empty list rather than erroring.

## Migration Plan

1. Add `examples/guest-openai` (user FaaS) and wire it into the example manifest with module target + dependencies.
2. Retarget `model-broker`'s register URL.
3. Remove the two system crates, their `manifest.toml` entries, and the two routes from `inject_feature_routes`.
4. Drop the stale `ai-list-model` slug from `has_ai`.

Rollback: revert the commit; the two system crates and their injection are restored. No persisted data migration is required — `guest-openai` reads the same `ai-models-registry` table the old registry wrote, so existing entries carry over.
