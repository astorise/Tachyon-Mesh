# Design Notes

## Group entity model

```text
state_dir/
├── alice.json                 # UserProfileRecord (existing layout, extended)
├── bob.json
├── groups/
│   ├── platform-admins.json   # GroupRecord (new)
│   └── viewers.json
├── pending-enrollments/
├── pending-logins/
└── registration-tokens/
```

`GroupRecord` is a flat document on disk:

```json
{
  "name": "platform-admins",
  "description": "Owns mesh seal and apply",
  "roles": ["admin"],
  "scopes": ["config-routing", "config-security"],
  "created_at": 1714780800,
  "updated_at": 1714780800
}
```

`UserProfileRecord` gains three optional fields, all with `serde(default)`
so existing on-disk records keep deserializing:

- `groups: Vec<String>` — group names this user belongs to.
- `disabled_at: Option<u64>` — Unix-seconds timestamp of disablement;
  `None` means active.
- `last_login_at: Option<u64>` — Unix-seconds timestamp of the last
  successful `finalize-login`.

## Effective roles and scopes at auth time

`finalize-login` and `validate-token` continue to return the user's
direct `roles`/`scopes`. To respect group membership without requiring a
WIT shape change to `auth-session`, the guest computes the **effective**
roles and scopes at finalization by unioning the user's direct lists
with the lists declared by every group they belong to. The resulting
`auth-session.roles` / `auth-session.scopes` are this union, so existing
authz callers see no behavioural change beyond "membership now grants
access".

If a referenced group is missing from disk, the guest skips it silently
rather than failing the login — group records can be deleted while users
still reference them. `update-user` does not validate that referenced
group names exist; the source of truth is the runtime check at login.

## Disable semantics

- `update-user` with `disabled: Some(true)` writes `disabled_at = now()`.
- `update-user` with `disabled: Some(false)` clears the field.
- `finalize-login` rejects with `"account disabled"` when the field is
  `Some`.
- An admin token cannot disable itself; the guest rejects the call when
  the target username equals the caller's subject (the caller subject is
  passed by `core-host` from the validated bearer token).

## Audit ring buffer

`core-host::AppState` gains an `iam_audit_log: Arc<RwLock<VecDeque<IamAuditEntry>>>`
capped at 1024 entries (oldest evicted). Every IAM admin handler appends
an entry:

```rust
struct IamAuditEntry {
    timestamp: u64,
    actor: String,        // subject from AuthClaims
    target_user: Option<String>,
    target_group: Option<String>,
    action: String,       // "user.disable", "user.enable", "user.delete",
                          // "user.update", "group.upsert", "group.delete"
    outcome: String,      // "ok" or short error tag
    detail: String,
}
```

`stage-login` / `finalize-login` events are also recorded with
`actor = username` so login traces show up in the same query. The
existing `validate-token` path is **not** logged (too hot a path).

`/admin/logs?user=<u>&lines=<N>` returns the most recent N entries
(default 50, max 500) where `target_user == u` or `actor == u`, ordered
newest-first. Without `user`, it returns all entries.

## REST surface

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/admin/iam/users` | list all users |
| PATCH | `/admin/iam/users/{username}` | update groups, roles, scopes, disabled |
| DELETE | `/admin/iam/users/{username}` | delete a user (rejects self) |
| GET | `/admin/iam/groups` | list all groups |
| POST | `/admin/iam/groups` | upsert a group (idempotent on `name`) |
| DELETE | `/admin/iam/groups/{name}` | delete a group |
| GET | `/admin/logs` | recent IAM audit entries (optionally filtered) |

All routes are mounted under the existing `admin_auth_middleware`. The
`/admin/logs` endpoint is also exposed without a `user` filter so the
existing `tail_logs` Tauri command keeps working.

## WIT compatibility

The change is additive: `wit/authn.wit` bumps from `1.0.0` to `1.1.0`.
Existing `validate-token`, `stage-user`, `finalize-enrollment`,
`stage-login`, `finalize-login`, `issue-pat`, and recovery functions keep
their signatures. The `wit-compat` workflow validates this is a minor
bump compatible with `1.0.x` consumers.

## UI shape

`<tachyon-users-panel>` is a new component routed at `/users`. Layout:

- top bar: search + "create group" trigger;
- table of users (username, status, groups, roles, last login, actions);
- inline action menu per row: enable/disable, edit groups, edit roles,
  delete, view audit;
- right rail showing the group catalog with role badges and member count;
- modal dialog for the audit-log filtered view, calling
  `tail_logs_for_user`.

The legacy `<tachyon-rbac-panel>` keeps its current "POST a per-role
policy" function but the route label moves to "RBAC Policy" so the user
panel owns "Users & Groups". The `nav.users` and `nav.rbac` keys are both
present in the i18n dictionary.
