## ADDED Requirements

### Requirement: System AuthN and Admin routes bypass sealed FaaS routing
The `core-host` HTTP router SHALL treat `/auth/*` and `/admin/*` paths as reserved system route namespaces before evaluating sealed FaaS routes from `integrity.lock`.

#### Scenario: Registered system auth route is served by core-host
- **WHEN** a client submits a request to a registered `/auth/*` route such as `/auth/login/stage`
- **THEN** `core-host` dispatches the request to the built-in AuthN handler
- **AND** the request is not resolved through the sealed FaaS route table

#### Scenario: Unknown system route does not report an integrity lock failure
- **WHEN** a client submits a request to an unregistered `/auth/*` or `/admin/*` path
- **THEN** `core-host` returns `404 Not Found`
- **AND** the response identifies the path as an unregistered system route
- **AND** the response does not say that the route is not sealed in `integrity.lock`

#### Scenario: Non-system guest route still requires sealing
- **WHEN** a client submits a request to an unregistered non-system path such as `/api/missing`
- **THEN** `core-host` continues to enforce sealed FaaS route lookup
- **AND** the request is rejected if the path is absent from `integrity.lock`
