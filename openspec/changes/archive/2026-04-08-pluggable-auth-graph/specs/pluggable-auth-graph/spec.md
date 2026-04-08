## ADDED Requirements

### Requirement: Targets can declare delegated credentials and optional middleware
The integrity manifest SHALL allow each target to declare the credential scopes it requires for itself and for downstream dependency delegation, and MAY declare a middleware target that executes before the primary handler.

#### Scenario: A target declares middleware and delegated credentials
- **WHEN** an operator defines a target with `middleware` and `requires_credentials`
- **THEN** the manifest parser accepts the target definition
- **AND** the runtime model preserves both fields for validation and request execution

### Requirement: Startup rejects dependency chains with missing delegated credentials
The host SHALL validate the resolved dependency graph at startup and fail initialization when a target does not declare every credential required by one of its dependencies.

#### Scenario: A caller omits a credential required by a dependency
- **WHEN** target `faas-a` depends on target `faas-b`
- **AND** `faas-b` requires credential `c2`
- **AND** `faas-a` does not declare `c2` in `requires_credentials`
- **THEN** host startup fails with a credential delegation validation error

### Requirement: Middleware can short-circuit target execution
The request handler SHALL execute the configured middleware target before the main target and SHALL return the middleware response immediately unless it succeeds with HTTP 200.

#### Scenario: Middleware denies a request
- **WHEN** a route has a configured middleware target
- **AND** the middleware returns a non-200 HTTP response
- **THEN** the host returns that middleware response to the client
- **AND** the main target is not instantiated

#### Scenario: Middleware allows a request
- **WHEN** a route has a configured middleware target
- **AND** the middleware returns HTTP 200
- **THEN** the host discards the middleware body
- **AND** request execution continues with the primary target
