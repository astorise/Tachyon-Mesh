# Async Logging

## Purpose
Define how Tachyon captures guest stdout and stderr off the request path, batches those log records in memory, and drains them through a dedicated logger component without coupling guest latency to blocking host I/O.
## Requirements
### Requirement: Guest stdout and stderr are captured asynchronously
The host SHALL redirect guest stdout and stderr into a bounded in-memory queue instead of performing synchronous disk or console writes on the request execution path.

#### Scenario: A guest emits a burst of logs
- **WHEN** a guest writes heavily to stdout or stderr during request handling
- **THEN** the host enqueues those log records asynchronously
- **AND** request execution does not block waiting for file or console I/O

### Requirement: Log export is delegated to a system logger component
The host SHALL batch queued guest log records and deliver them to a dedicated system logger component for persistence or export.

#### Scenario: The logger route is sealed
- **WHEN** the integrity manifest includes the system logger route
- **THEN** the host drains queued records in batches
- **AND** invokes the logger component with serialized log entries

### Requirement: Legacy preview1 guests no longer depend on synchronous stdout temp files
Legacy guest execution SHALL preserve non-log stdout response data while diverting structured log lines into the asynchronous queue.

#### Scenario: A legacy guest prints structured logs and a small response
- **WHEN** a preview1 guest emits many structured JSON log lines followed by a plain-text response
- **THEN** the plain-text response is still returned to the caller
- **AND** the structured log lines are exported asynchronously through the logger pipeline

### Requirement: Guest logs are captured asynchronously off the request path
The host SHALL redirect guest stdout and stderr into an asynchronous in-memory buffer that a dedicated system logger drains without blocking request execution.

#### Scenario: A guest emits a burst of logs
- **WHEN** a guest writes heavily to stdout or stderr during request handling
- **THEN** the host enqueues those log records asynchronously
- **AND** a dedicated logger component drains and exports them without coupling request latency to file or console I/O

