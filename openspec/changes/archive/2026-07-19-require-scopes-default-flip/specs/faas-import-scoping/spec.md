## ADDED Requirements

### Requirement: Node-level `require_scopes` flag enforces explicit scopes at submission

The node configuration SHALL expose a `require_scopes` boolean, defaulting to `false`. When `true`, manifest validation SHALL reject at submission time any route whose `scopes` block is absent, or whose `scopes` resolve to `allow-all`. Rejection errors SHALL name the offending route and SHALL name `tachyon_suggest_scopes` as the tool that produces a starting scopes configuration for that route.

#### Scenario: Route missing a scopes block under require_scopes=true
- **WHEN** `require_scopes` is `true`
- **AND** a route in the manifest has no `scopes` block
- **THEN** manifest validation MUST reject the manifest
- **AND** the error MUST name the route's path
- **AND** the error MUST name `tachyon_suggest_scopes` as the remediation tool

#### Scenario: Route resolves to allow-all under require_scopes=true
- **WHEN** `require_scopes` is `true`
- **AND** a route's `scopes` block resolves to `allow-all` (either the literal string `"allow-all"` or an equivalent wildcard shape)
- **THEN** manifest validation MUST reject the manifest
- **AND** the error MUST name the route's path
- **AND** the error MUST name `tachyon_suggest_scopes` as the remediation tool

#### Scenario: Route carries explicit non-allow-all scopes under require_scopes=true
- **WHEN** `require_scopes` is `true`
- **AND** every route in the manifest has an explicit `scopes` block that does not resolve to `allow-all`
- **THEN** manifest validation MUST succeed

#### Scenario: Default require_scopes=false preserves existing manifests
- **WHEN** `require_scopes` is `false` (the default; the flag is absent from the manifest)
- **AND** a route has no `scopes` block
- **THEN** manifest validation MUST succeed (the route resolves to `allow-all` at runtime, per the `faas-import-scoping` migration-default requirement)

### Requirement: Explicit `require_scopes: false` remains a supported operator choice

`require_scopes: false` SHALL remain a valid, permanent manifest setting regardless of what value the field defaults to in later changes. A node that explicitly sets `require_scopes: false` SHALL continue to accept manifests with absent or allow-all `scopes` blocks, independent of any future change to the field's default.

#### Scenario: Cluster pins require_scopes=false ahead of a future default change
- **WHEN** a node's manifest explicitly sets `require_scopes: false`
- **AND** a route has no `scopes` block
- **THEN** manifest validation MUST succeed, regardless of the compiled-in default value of `require_scopes`
