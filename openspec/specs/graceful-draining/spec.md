# graceful-draining Specification

## Purpose
TBD - created by archiving change graceful-draining. Update Purpose after archive.
## Requirements
### Requirement: Reloaded route generations drain in-flight work before destruction
The host SHALL keep the previous route generation in a draining state until in-flight work finishes or a safety timeout expires.

#### Scenario: A new route generation replaces an older one
- **WHEN** a reload activates a new generation for a route
- **THEN** new traffic is sent only to the active generation
- **AND** the previous generation remains alive until its in-flight counter reaches zero or the draining timeout is exceeded

