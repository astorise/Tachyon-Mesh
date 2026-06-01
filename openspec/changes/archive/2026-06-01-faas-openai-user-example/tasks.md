## 1. New `guest-openai` user FaaS

- [x] 1.1 Create `examples/guest-openai` crate (Cargo.toml mirroring an existing `examples/guest-*` cdylib component) and add it to the workspace members
- [x] 1.2 Generate bindings with `wit_bindgen::generate!` for world `faas-guest` (imports `kv-partition`, exports `handler`)
- [x] 1.3 Port the registry logic from `system-faas-ai-list-model`: register / list-models / deregister against the `ai-models-registry` `kv-partition` table — WITHOUT the `thread_local` `MODELS_CACHE` (read the table on every list)
- [x] 1.4 Port the OpenAI reshaping from `system-faas-openai-adapter`: `GET /v1/models` reads the table directly and maps records to `{ id, object: "model", owned_by: "tachyon-mesh" }` inside `{ object: "list", data: [...] }`
- [x] 1.5 Implement `POST /v1/chat/completions` as a `501` stub with an OpenAI-shaped error body
- [x] 1.6 Keep the register/list/deregister route paths stable and document them as constants (`/internal/guest-openai/register`, `/internal/guest-openai/deregister/{alias}`)
- [x] 1.7 Unit tests: malformed register payload rejected, list returns OpenAI shape, chat completions returns 501, deregister strips alias correctly

## 2. Build wiring

- [x] 2.1 Add `guest-openai` to `scripts/build-guest-artifacts.sh` so the `.wasm` is produced alongside the other `guest-*` modules
- [x] 2.2 Confirm `examples/guest-openai` builds to a component (`cargo build -p guest-openai --target wasm32-wasip2 --release`)

## 3. Example manifest + topology

- [x] 3.1 Add `guest-openai` user routes to `examples/guest-examples/manifest.json`: `/v1/models`, `/v1/chat/completions`, `/internal/guest-openai/register`, each with `targets.module = guest-openai`
- [x] 3.2 Routes use distinct names (`openai-models`, `openai-chat`, `openai-registry`) — the validator rejects same-name/same-version routes, so the OpenAI layer renders as sibling endpoint→custom-wasm nodes sharing the `guest-openai` asset source (no synthetic dependency edge fabricated)
- [x] 3.3 Declare a `scopes.kv` grant for the `ai-models-registry` table on each `guest-openai` route
- [x] 3.4 Verify the manifest passes validation — added `guest_openai_example_routes_validate_with_kv_scope` in `core-host/.../tests/config_validation.rs` exercising `validate_integrity_config` on the three routes (green)

## 4. `model-broker` retarget

- [x] 4.1 Repoint the register URL in `systems/system-faas-model-broker/src/lib.rs` to `http://mesh/internal/guest-openai/register` (const + fn renamed to `MODEL_REGISTRY_REGISTER_URL` / `notify_model_registry`)
- [x] 4.2 Egress is authorized: `model-broker` is a system caller and the target is a sealed internal route (`resolve_outbound_http_target` → `by_path` Internal); no extra dependency/resource needed

## 5. Decommission system FaaS

- [x] 5.1 Remove the `ai-list-model` / `ai-openai-adapter` `to_inject.push(...)` lines from `inject_feature_routes` (keep `model-broker`)
- [x] 5.2 Remove the `ai-list-model` and `ai-openai-adapter` `[[system]]` entries from `systems/manifest.toml`
- [x] 5.3 Delete the `systems/system-faas-openai-adapter` and `systems/system-faas-ai-list-model` crates, remove from workspace members and `scripts/build-feature-system-faas.sh`
- [x] 5.4 Drop the stale `slugs.contains("ai-list-model")` term from `has_ai` in `tachyon-client/src/lib.rs::get_cluster_features`
- [x] 5.5 Grep for residual references and clean up (gateway `OPENAI_ADAPTER_ROUTE` rewrite removed; `/v1/*` now passes through as OLTP)

## 6. Verification

- [x] 6.1 Affected crates green: `guest-openai` build+test+clippy, `system-faas-gateway` test, `system-faas-model-broker` wasm build, `core-host` check + targeted test, `tachyon-client` check; `cargo fmt --check` clean
- [~] 6.2 Topology from the example manifest shows the OpenAI endpoint→custom-wasm nodes without `ai-inference` — verified structurally via the `validate_integrity_config` test + topology edge logic; live render not run (no connected node in this environment)
- [~] 6.3 End-to-end register → `GET /v1/models` → deregister — guest logic unit-tested; full live e2e requires a running node + sealed manifest (deferred to a node environment)
- [x] 6.4 `openspec validate faas-openai-user-example --strict` passes
