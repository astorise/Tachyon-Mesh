## ADDED Requirements

### Requirement: Route-scoped dynamic model bindings

The integrity manifest SHALL support dynamic model bindings whose model content
is resolved from the managed broker model directory at runtime. A dynamic
binding SHALL authorize only the route that declares it and SHALL NOT require a
static model path.

#### Scenario: Dynamic binding omits static path

- **WHEN** integrity configuration normalization receives a model binding with
  `dynamic: true` and an empty path
- **THEN** normalization SHALL preserve the dynamic flag and accept the binding

#### Scenario: Static binding still requires path

- **WHEN** integrity configuration normalization receives a static model
  binding with an empty path
- **THEN** normalization SHALL reject the binding

#### Scenario: Dynamic authorization remains route-scoped

- **GIVEN** a dynamic alias is bound to the OpenAI chat route
- **WHEN** another route attempts to load the same alias without declaring it
- **THEN** the host SHALL reject that route's request as not sealed
