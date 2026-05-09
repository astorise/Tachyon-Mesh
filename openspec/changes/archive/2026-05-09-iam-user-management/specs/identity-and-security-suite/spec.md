# identity-and-security-suite

## ADDED Requirements

### Requirement: Group Entity
The authn guest SHALL persist groups as discrete records under
`groups/<name>.json` in the auth state directory. Each group SHALL store
a name, an optional description, a list of roles, a list of scopes, and
creation/update timestamps.

#### Scenario: Group records are persisted independently of users
- **WHEN** an admin upserts a group via `upsert-group`
- **THEN** the guest writes a `groups/<name>.json` document
- **AND** the file persists across host restarts
- **AND** deleting the group removes only that file without touching user
  records

#### Scenario: Group names follow a strict normalization
- **WHEN** an admin upserts a group with mixed-case or whitespace in the
  name
- **THEN** the guest stores the name lowercased and trimmed
- **AND** the guest rejects names that contain characters other than
  ASCII letters, digits, `-`, `_`, or `.`

### Requirement: User Group Membership
A `UserProfileRecord` SHALL include a `groups` field listing the names
of the groups the user belongs to. The authn guest SHALL union the
user's direct roles and scopes with those of every referenced group when
issuing an authentication session.

#### Scenario: Effective roles include group memberships
- **GIVEN** user `alice` belongs to group `platform-admins` whose roles
  include `admin`
- **WHEN** `alice` finalizes a login
- **THEN** the issued session includes `admin` in its roles
- **AND** the authz layer therefore grants admin-only routes

#### Scenario: Missing group references are tolerated
- **GIVEN** user `bob` references a group `legacy-ops` that no longer
  exists on disk
- **WHEN** `bob` finalizes a login
- **THEN** the guest skips the missing group silently
- **AND** the issued session reflects only the remaining valid roles and
  scopes

### Requirement: User Disable Lifecycle
The `UserProfileRecord` SHALL include a `disabled_at` timestamp. The
authn guest SHALL refuse `stage-login` and `finalize-login` for any user
whose `disabled_at` is set, and an admin SHALL NOT be able to disable
their own account.

#### Scenario: Disabled accounts cannot stage a login
- **GIVEN** an admin disables user `bob`
- **WHEN** `bob` submits valid credentials to `stage-login`
- **THEN** the guest returns `account is disabled`
- **AND** no staged session is created

#### Scenario: Re-enabling clears the disable timestamp
- **WHEN** an admin issues `update-user` with `disabled: Some(false)`
- **THEN** the user's `disabled_at` field is cleared
- **AND** the user can stage and finalize a login again

#### Scenario: Self-disable is rejected
- **WHEN** the actor of `update-user` equals the target username and
  `disabled: Some(true)` is requested
- **THEN** the guest returns `operators cannot disable their own account`

### Requirement: User Last Login Tracking
The authn guest SHALL update `last_login_at` on each successful
`finalize-login` and SHALL surface the value through the
`user-summary.last-login-at` field.

#### Scenario: Successful login updates the timestamp
- **GIVEN** user `alice` has never signed in
- **WHEN** `alice` finalizes a login successfully
- **THEN** her stored `last_login_at` equals the unix timestamp of the
  finalization
- **AND** subsequent `list-users` calls return the new value

### Requirement: User Lifecycle Operations
The authn guest SHALL expose `list-users`, `update-user`, and
`delete-user` functions through `wit/authn.wit`, with `update-user`
supporting additive and subtractive changes to groups, roles, scopes,
and the disabled flag.

#### Scenario: List returns every enrolled user
- **WHEN** an admin invokes `list-users`
- **THEN** the response contains one summary per enrolled user
- **AND** pending enrollments are excluded

#### Scenario: Update merges add and remove deltas
- **GIVEN** user `alice` has groups `[ops]` and roles `[viewer]`
- **WHEN** an admin updates `alice` with `add_groups=[platform-admins]`
  and `remove_roles=[viewer]`
- **THEN** the resulting record has groups `[ops, platform-admins]` and
  roles `[]`

#### Scenario: Delete refuses self deletion
- **WHEN** the actor of `delete-user` equals the target username
- **THEN** the guest returns `operators cannot delete their own account`

### Requirement: Group Lifecycle Operations
The authn guest SHALL expose `list-groups`, `upsert-group`, and
`delete-group` functions. `upsert-group` SHALL be idempotent on the
group name; `delete-group` SHALL refuse to delete groups that do not
exist.

#### Scenario: Upsert normalizes and persists
- **WHEN** an admin upserts a group with description, roles, and scopes
- **THEN** the response contains the normalized name, the supplied
  description, the deduplicated roles and scopes, the count of users
  currently referencing the group, and creation/update timestamps

#### Scenario: Delete removes the group file
- **WHEN** an admin deletes an existing group
- **THEN** the corresponding `groups/<name>.json` file is removed
- **AND** subsequent `list-groups` calls do not return that group

### Requirement: Core Host IAM Endpoints
The core host SHALL expose `/admin/iam/users`, `/admin/iam/users/{username}`,
`/admin/iam/groups`, and `/admin/iam/groups/{name}` under the existing
admin auth middleware, mapping HTTP verbs to the IAM operations defined
above.

#### Scenario: PATCH on a user routes to update-user
- **GIVEN** an admin holds a valid bearer token
- **WHEN** they `PATCH /admin/iam/users/alice` with a JSON
  `UpdateUserRequest`
- **THEN** the host calls the guest's `update-user`
- **AND** the response is the resulting user summary

#### Scenario: Unauthenticated callers receive 401
- **WHEN** any IAM admin route is called without a bearer token
- **THEN** the response status is 401
- **AND** the audit ring buffer records nothing for the caller

### Requirement: IAM Audit Ring Buffer
The core host SHALL maintain an in-memory ring buffer of IAM lifecycle
events bounded at 1024 entries. Each successful or failed IAM admin
operation, plus `stage-login` and `finalize-login` events, SHALL append
an entry recording the actor, action, optional target user or group,
outcome, and a free-form detail field.

#### Scenario: Successful actions emit `outcome=ok`
- **WHEN** an admin upserts a group successfully
- **THEN** the ring buffer contains an entry with
  `action=group.upsert`, `outcome=ok`, and `target_group` set to the
  group's name

#### Scenario: Failures emit `outcome=error`
- **WHEN** an admin tries to delete their own account and the guest
  rejects it
- **THEN** the ring buffer contains an entry with
  `action=user.delete`, `outcome=error`, and the rejection message in
  the detail field
