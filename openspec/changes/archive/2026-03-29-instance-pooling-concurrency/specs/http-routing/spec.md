## ADDED Requirements

### Requirement: Host enforces per-route concurrency budgets
The `core-host` HTTP gateway SHALL enforce the sealed `max_concurrency` value for each route with
an asynchronous semaphore so guest execution is queued safely instead of oversubscribing the host.

#### Scenario: Request waits for route capacity
- **WHEN** a request arrives for a sealed route whose current in-flight executions are below
  `max_concurrency`
- **THEN** the host acquires a permit for that route before guest execution starts
- **AND** the permit remains held until the request handler completes
- **AND** the request continues through the normal guest execution path

#### Scenario: Request times out waiting for route capacity
- **WHEN** a request waits longer than five seconds for a permit on the target route
- **THEN** the host does not execute the guest
- **AND** the HTTP response status is `503 Service Unavailable`
- **AND** the response explains that the route is currently saturated
