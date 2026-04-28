# Implementation Tasks

## Phase 1: Wasm Emitter (`system-faas-authz`)
- [ ] Open `systems/system-faas-authz/src/lib.rs`.
- [ ] Identify the endpoints responsible for token revocation and user role updates.
- [ ] Inject an event emission call (`emit_event`) immediately after a successful database/KV write, targeting the `system.authz` topic.

## Phase 2: Host Subscriber (`core-host`)
- [ ] In `core-host/src/auth.rs` (or where the auth cache is defined), expose an invalidation method (e.g., `invalidate_token(hash)`).
- [ ] In `core-host/src/data_events.rs` (or the central event router), add a listener for `system.authz` events.
- [ ] Wire the listener to call the invalidation methods on the active `AuthCache` instance.

## Phase 3: Validation
- [ ] **Test Revocation:** Log in via Tachyon UI, cache the token by making a few requests. Use another session to revoke the first token. The very next request from the first session MUST yield a `401 Unauthorized`.
- [ ] **Test Role Change:** Change a user's role from `admin` to `user`. Verify that subsequent requests to admin-only `/system/*` routes immediately return `403 Forbidden`.