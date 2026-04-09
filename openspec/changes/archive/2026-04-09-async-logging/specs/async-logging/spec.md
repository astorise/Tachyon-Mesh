## ADDED Requirements

### Requirement: Guest logs are captured asynchronously off the request path
The host SHALL redirect guest stdout and stderr into an asynchronous in-memory buffer that a dedicated system logger drains without blocking request execution.

#### Scenario: A guest emits a burst of logs
- **WHEN** a guest writes heavily to stdout or stderr during request handling
- **THEN** the host enqueues those log records asynchronously
- **AND** a dedicated logger component drains and exports them without coupling request latency to file or console I/O
