# iam-webcomponent

## ADDED Requirements

### Requirement: Users And Groups Panel
Tachyon-UI SHALL expose a `<tachyon-users-panel>` web component routed
at `/users` that lists every enrolled user, their group memberships,
their roles, their last login timestamp, and their account status, with
inline actions to disable, re-enable, edit groups, edit roles, view
audit history, and delete a user.

#### Scenario: Panel mirrors backend list
- **WHEN** the operator opens the `users` route
- **THEN** the panel calls `iam_list_users` and renders one row per
  user
- **AND** each row shows status, groups, roles, last login, and an
  actions menu

#### Scenario: Inline action invokes the corresponding command
- **WHEN** the operator clicks "Disable" on a row
- **THEN** the panel calls `iam_update_user` with
  `{ disabled: true }`
- **AND** refreshes the table on success

#### Scenario: Audit modal shows the per-user history
- **WHEN** the operator clicks "View audit" on a row
- **THEN** the panel calls `fetch_user_audit_log` for that username
- **AND** displays the entries in a modal with timestamp, action,
  outcome, and detail columns

### Requirement: Group Catalog
The `<tachyon-users-panel>` component SHALL render a group catalog
alongside the user table that lists every group with its description,
roles, scopes, and member count, plus a form to create or update a
group and a control to delete a group.

#### Scenario: Catalog reflects backend state
- **WHEN** the panel loads
- **THEN** it calls `iam_list_groups`
- **AND** renders one card per group with role badges and scope badges

#### Scenario: Form creates and edits the same way
- **WHEN** the operator submits the create form with a name, roles, and
  scopes
- **THEN** the panel calls `iam_upsert_group`
- **AND** subsequently selecting the same group's "Edit" control
  populates the form with the stored values
