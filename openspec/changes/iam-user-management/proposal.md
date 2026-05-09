# Proposal: IAM User and Group Management

## 1. Context

The Tachyon-UI exposes an authentication overlay and a thin RBAC panel that
posts a JSON policy per role, but operators cannot:

- list the users registered against a node;
- add a user to or remove a user from a group;
- create, edit, or delete groups as a separate IAM entity;
- disable or re-enable a user account;
- inspect the audit history of a specific user.

The current data model in `system-faas-authn` only stores `roles` and
`scopes` directly on a `UserProfileRecord`. There is no `Group` entity, no
`disabled_at` flag, no `last_login_at` timestamp, and no per-user audit
trail. Several components (`<tachyon-rbac-panel>`,
`<tachyon-identity-panel>`) only generate one-off configuration payloads
without any read or list capability. The `tachyon-client::iam_list_users`
function is a stub that returns the active operator under hardcoded
"admin"/"ops" groups rather than data fetched from the backend.

## 2. Solution

Introduce a real IAM model with users, groups, and audit logging:

1. **Group entity.** Persist groups in `system-faas-authn` as discrete
   records under `groups/<name>.json`. A group owns a list of `roles` and
   `scopes`. Users gain a `groups: Vec<String>` field; their effective
   roles and scopes at authentication time are the union of their direct
   `roles`/`scopes` and the roles/scopes inherited from their groups.
2. **User lifecycle fields.** `UserProfileRecord` gains `groups`,
   `disabled_at: Option<u64>`, and `last_login_at: Option<u64>`, all with
   `serde(default)` so existing on-disk records keep working. Login
   finalization is rejected when `disabled_at` is set, and successful
   logins update `last_login_at`.
3. **WIT contract bump.** `wit/authn.wit` moves from
   `tachyon:identity@1.0.0` to `tachyon:identity@1.1.0` with new
   functions: `list-users`, `update-user`, `delete-user`, `list-groups`,
   `upsert-group`, `delete-group`. The existing functions are unchanged.
4. **Core-host endpoints.** `core-host` exposes a new IAM admin surface
   (`/admin/iam/users`, `/admin/iam/users/{username}`,
   `/admin/iam/groups`, `/admin/iam/groups/{name}`) gated by the existing
   admin auth middleware. It also adds a server-side audit ring buffer
   surfaced through `GET /admin/logs?user=<username>&lines=<N>` so
   operators can inspect a specific user's recent IAM events.
5. **Client + Tauri bridge.** `tachyon-client` gains typed wrappers for
   each IAM endpoint and the audit-log query. `tachyon-ui` exposes them as
   Tauri commands.
6. **Users panel.** A new `<tachyon-users-panel>` web component renders
   the user list, group memberships, role chips, and account status
   (active/disabled), with inline actions to disable/re-enable, edit group
   memberships, edit roles, delete users, manage groups, and view a
   user's audit history. The legacy `<tachyon-rbac-panel>` form is kept
   for ad-hoc per-role policy edits but the route is renamed to make
   their distinct purposes obvious.

## 3. Non-goals

- Replacing the JSON policy editor in `<tachyon-rbac-panel>` with a fully
  visual policy designer. The new `<tachyon-users-panel>` covers user/
  group lifecycle; per-role policy authoring remains its own surface.
- Implementing federated identity providers (OIDC/SAML).
- Backfilling structured trace logs for non-IAM events. The audit ring
  buffer captures IAM lifecycle events specifically; broader observability
  remains under `compute-observability`.
- Migrating the existing on-disk `UserProfileRecord` files to a new
  layout. New fields are additive with `serde(default)`.
