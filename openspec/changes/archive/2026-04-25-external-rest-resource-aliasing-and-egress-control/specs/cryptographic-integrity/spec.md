## ADDED Requirements

### Requirement: Integrity manifest seals logical resource aliases
The signed `integrity.lock` payload SHALL allow a top-level `resources` map that binds a logical
alias to either an internal sealed route reference or an external HTTPS endpoint plus an HTTP
method allow-list.

#### Scenario: Host accepts a sealed internal resource alias
- **WHEN** `core-host` starts with a signed manifest containing `resources.inventory-api` of
  `type = "internal"`, `target = "inventory"`, and `version_constraint = "^1.2.0"`
- **THEN** integrity validation succeeds only if a compatible sealed route is present
- **AND** the normalized alias is retained as signed runtime configuration

#### Scenario: Host accepts a sealed external resource alias
- **WHEN** `core-host` starts with a signed manifest containing `resources.payment-gateway` of
  `type = "external"`, `target = "https://api.stripe.com/v1"`, and
  `allowed_methods = ["POST"]`
- **THEN** integrity validation succeeds
- **AND** the runtime retains the external target and normalized method allow-list from the signed
  manifest

#### Scenario: Host rejects ambiguous or unsafe resource aliases
- **WHEN** a sealed manifest defines a resource name that collides with a route name, omits the
  external method allow-list, or points an external resource at a non-HTTPS target
- **THEN** integrity validation fails before the host starts serving traffic
