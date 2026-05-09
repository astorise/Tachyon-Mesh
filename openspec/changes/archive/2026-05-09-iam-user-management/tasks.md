# Implementation Tasks

## 1. WIT Contract
- [x] 1.1 Bump `wit/authn.wit` package to `tachyon:identity@1.1.0`.
- [x] 1.2 Add `user-summary`, `user-update`, `group-summary`,
       `group-input` records and the six new functions.

## 2. Authn Guest
- [x] 2.1 Extend `UserProfileRecord` with `groups`, `disabled_at`,
       `last_login_at` (all `serde(default)`).
- [x] 2.2 Implement `list-users` by enumerating top-level `*.json` files
       in the auth state directory and projecting `UserSummary`.
- [x] 2.3 Implement `update-user`, `delete-user`, `list-groups`,
       `upsert-group`, `delete-group`.
- [x] 2.4 Reject `finalize-login` when `disabled_at` is set; update
       `last_login_at` on success.
- [x] 2.5 Compute effective roles and scopes at login by unioning the
       user record with every referenced group record.

## 3. Core Host
- [x] 3.1 Wrap the new WIT calls in `AuthManager`.
- [x] 3.2 Add the IAM admin routes under the existing admin middleware.
- [x] 3.3 Add an in-memory IAM audit ring buffer in `AppState` and
       record an entry from each IAM handler and login finalization.
- [x] 3.4 Add `GET /admin/logs` returning the audit buffer with
       optional `user` and `lines` query parameters.

## 4. Tachyon Client + Tauri Commands
- [x] 4.1 Add typed wrappers (`iam_list_users`, `iam_update_user`,
       `iam_delete_user`, `iam_list_groups`, `iam_upsert_group`,
       `iam_delete_group`, `tail_logs_for_user`).
- [x] 4.2 Replace the stubbed `iam_list_users` implementation with the
       remote call.
- [x] 4.3 Register the new Tauri commands in `tachyon-ui::main`.

## 5. UI
- [x] 5.1 Create `<tachyon-users-panel>` with the user table, group
       catalog, and inline action menu.
- [x] 5.2 Wire group create / edit / delete flows.
- [x] 5.3 Wire the per-user audit modal to `tail_logs_for_user`.
- [x] 5.4 Register the `users` route in `ComponentRegistry`.
- [x] 5.5 Add the `nav.users`, `users.*`, and `groups.*` i18n keys (en/fr).

## 6. Documentation
- [x] 6.1 Capture the new requirements in the
       `identity-and-security-suite`, `iam-webcomponent`, and
       `compute-observability` capability deltas.

## 7. Integration Tests
- [x] 7.1 Guest scenario tests in `system-faas-authn` covering
       `list-users`, `update-user` (add/remove deltas, self-disable
       refusal), `delete-user` (self-delete refusal, on-disk removal),
       `disabled` lifecycle (stage-login refusal, re-enable clears),
       `last-login-at` update, group role union at login, missing
       group tolerated, `upsert-group` normalization and dedup,
       invalid name rejection, member counting, and `delete-group`
       removal.
- [x] 7.2 Audit ring buffer unit tests covering newest-first ordering,
       target_user filter, actor filter, case-insensitive matching,
       lines clamp to 500 (and to 1 when zero), empty-string user
       filter behaviour, and ring eviction at capacity.
- [x] 7.3 Core-host endpoint surface tests covering 401 on each IAM
       route without a bearer, direct invocations of
       `audit_log_handler` for filtering and clamp behaviour, and
       camel-case wire format for `IamUserSummaryResponse`.
