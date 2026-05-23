## ADDED Requirements

### Requirement: Route detail view displays a Scopes panel with per-category glob editors
The Tachyon-UI SHALL provide a Scopes panel in the route detail view that lists the ten scope categories (secrets, kv, vector, http, routing, outbox, storage, training, bridge, graph) and lets the operator edit glob patterns for each.

#### Scenario: Scopes panel shows current scope configuration
- **WHEN** a user opens the detail view of a route
- **THEN** the Scopes panel renders each category as a collapsible row
- **AND** categories with no patterns show a "Not granted (interface not linked)" label in grey
- **AND** categories with patterns list each glob on a separate chip

#### Scenario: Operator adds a glob pattern to a category
- **WHEN** the operator clicks "Add pattern" in the kv category row
- **THEN** an inline text input appears accepting a glob string (e.g. `tenant-a/**`)
- **WHEN** the operator submits a valid glob
- **THEN** the pattern appears as a chip in that category row
- **AND** the "Save scopes" button becomes active

#### Scenario: Operator removes a glob pattern from a category
- **WHEN** the operator clicks the × on a pattern chip
- **THEN** the chip is removed from the row
- **AND** if all patterns in a category are removed the row returns to "Not granted" state

#### Scenario: Operator saves scopes via the manifest API
- **WHEN** the operator clicks "Save scopes"
- **THEN** the UI builds a `scopes:` block from the current panel state
- **AND** submits it via `POST /admin/manifest`
- **AND** a success toast confirms the manifest was sealed
- **AND** the panel refreshes from the new manifest response

#### Scenario: Save fails with an API error
- **WHEN** the `POST /admin/manifest` call returns a non-2xx status
- **THEN** the panel shows an error toast with the server message
- **AND** the local panel state is preserved so the operator can retry or correct

### Requirement: Scopes panel enforces routing tuple syntax
The routing category input SHALL require the format `<route-path-glob> -> <destination-glob>` and reject entries missing the arrow separator.

#### Scenario: Operator enters a routing pattern without destination
- **WHEN** the operator types `/api/*` without ` -> ` in the routing category input
- **THEN** the input shows an inline validation error: "Routing pattern must include a destination: /api/* -> /target/*"
- **AND** the "Save scopes" button remains disabled

#### Scenario: Valid routing tuple is accepted
- **WHEN** the operator types `/api/v2/* -> /internal/v2/*`
- **THEN** no validation error appears
- **AND** the pattern is accepted as a chip

### Requirement: Scopes panel surfaces an allow-all warning
When the current scope resolves to allow-all (missing block or explicit sentinel), the Scopes panel SHALL display a prominent warning badge.

#### Scenario: Route with no scopes block shows allow-all badge
- **WHEN** the operator opens a route detail view for a route with no `scopes:` key
- **THEN** the Scopes panel header shows an amber "ALLOW ALL" badge
- **AND** a help text explains: "This deployment grants every WIT import. Add explicit patterns to tighten access."

#### Scenario: Operator sets explicit scopes to dismiss the allow-all badge
- **WHEN** the operator adds at least one pattern to any category and saves
- **THEN** the "ALLOW ALL" badge disappears from the panel header

### Requirement: Glob patterns are validated inline before submission
The UI SHALL validate each pattern string using glob syntax rules before enabling the Save button.

#### Scenario: Invalid glob characters are rejected
- **WHEN** the operator enters a pattern containing an unbalanced `{` bracket
- **THEN** the input field shows an inline error: "Invalid glob pattern"
- **AND** the pattern is not added to the chip list

#### Scenario: Valid glob patterns with wildcards are accepted
- **WHEN** the operator enters `db/prod/**` or `https://api.example.com/*`
- **THEN** the pattern is accepted without error
