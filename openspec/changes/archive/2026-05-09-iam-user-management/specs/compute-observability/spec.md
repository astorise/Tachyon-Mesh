# compute-observability

## ADDED Requirements

### Requirement: Per-User IAM Audit Logs
The core host SHALL expose `GET /admin/logs` returning the IAM audit
ring buffer in newest-first order, with optional `user` and `lines`
query parameters. The `user` filter SHALL match entries whose
`target_user` or `actor` equals the supplied value. The `lines` filter
SHALL clamp between 1 and 500, defaulting to 50 when omitted.

#### Scenario: User filter returns matching entries
- **GIVEN** the audit buffer contains entries for several users
- **WHEN** an admin requests `GET /admin/logs?user=alice`
- **THEN** the response contains only entries whose `actor` or
  `target_user` is `alice`
- **AND** the response is ordered newest-first

#### Scenario: Lines parameter clamps to 500
- **WHEN** an admin requests `GET /admin/logs?lines=10000`
- **THEN** the response contains at most 500 entries
- **AND** no error is returned

#### Scenario: Default response without filters
- **WHEN** an admin requests `GET /admin/logs` with no parameters
- **THEN** the response contains the most recent 50 audit entries
- **AND** the entries are not filtered by user
