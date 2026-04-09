# l7-resiliency Specification

## Purpose
Define how Tachyon applies route-level timeout and retry behavior only when the host is built with the optional resiliency feature, so the default binary keeps zero resiliency overhead.

## Requirements
### Requirement: Route-level resiliency policies are feature-gated and declarative
The platform SHALL allow routes to declare timeout and retry policies that are only enforced when the host is compiled with the resiliency feature enabled.

#### Scenario: A resiliency-enabled route defines timeout and retry policy
- **WHEN** the host is built with resiliency support and loads a route with timeout or retry settings
- **THEN** it applies the configured middleware chain for that route
- **AND** avoids introducing resiliency overhead when the feature is disabled
