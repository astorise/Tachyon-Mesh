# Implementation Tasks

## Phase 1: Wasm Emitter (`system-faas-authz`)
- [ ] Wiring the existing `system-faas-authz` mutation paths to call
      `AuthzPurgeEvent::enqueue` is left as a small follow-up. The enqueue
      helper, the redb outbox table, and the host-side subscriber are all
      in place; the FaaS just needs to import the helper and call it.

## Phase 2: Host Subscriber (`core-host`)
- [x] `AuthDecisionCache` (moka, capped at 16 384 entries with a 5-minute
      time-to-idle) lives on `AuthManager` in `core-host/src/auth.rs`.
- [x] `AuthManager::authorize_request` consults the cache before re-running
      the authn+authz components; only positive decisions are cached.
- [x] `spawn_authz_purge_subscriber` polls `authz_purge_outbox` every 250 ms
      (batches of 64), calls `apply_authz_purge` to evict the matching
      entries, and deletes the outbox row.
- [x] `apply_authz_purge` handles three event kinds: token revoke
      (invalidate_token), role update / user ban (invalidate_subject).

## Phase 3: Validation
- [x] Four unit tests cover round-trip get/put, token-scoped eviction,
      subject-scoped eviction, and a full enqueue → peek_outbox → apply
      round trip through redb.
- [ ] Manual: revoke a PAT in one session, observe that subsequent requests
      using that PAT yield 401 Unauthorized within ~1 s. Left for the
      homelab smoke test once `system-faas-authz` is wired to the enqueue.
