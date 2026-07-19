# tachyon-ui-error-logging Specification

## Purpose
TBD - created by archiving change tachyon-ui-error-logging. Update Purpose after archive.
## Requirements
### Requirement: Desktop error logging is configured through a versioned local application file
Tachyon-UI SHALL load a versioned JSON configuration file named
`tachyon-ui.config.json` from its local application-data directory and SHALL
create a default configuration when that file does not yet exist.

#### Scenario: First application launch creates default logging configuration
- **WHEN** Tachyon-UI starts without an existing local application configuration file
- **THEN** it creates `tachyon-ui.config.json` with `schemaVersion` set to `1`
- **AND** it configures logging with level `info`, file `logs/tachyon-ui.jsonl`, maximum size `5242880` bytes, and `5` retained files

#### Scenario: Operator changes the persisted logging settings
- **WHEN** the operator edits valid logging settings in `tachyon-ui.config.json` and restarts Tachyon-UI
- **THEN** the application uses the configured log level, relative log file path, maximum file size, and retained file count for the new process

#### Scenario: Configuration attempts to write logs outside local application data
- **WHEN** `logging.file` contains an absolute path or parent-directory traversal
- **THEN** Tachyon-UI rejects the configuration during startup
- **AND** it does not initialize a writer at the escaped path

### Requirement: Desktop application log entries are structured, filtered, and bounded on disk
Tachyon-UI SHALL persist enabled desktop application log events as JSON Lines
records with timestamp, severity, source, and message fields, and SHALL bound
disk usage through configured severity filtering and file rotation.

#### Scenario: Enabled error event is persisted as a structured record
- **GIVEN** the configured minimum level enables `error` records
- **WHEN** Tachyon-UI records a desktop error event
- **THEN** the active log file receives one JSON Lines record containing `timestampUnixMs`, `level`, `source`, and `message`

#### Scenario: Event is below the configured level
- **GIVEN** `logging.level` is `warn`
- **WHEN** Tachyon-UI emits an `info` event
- **THEN** the event is not appended to the active log file

#### Scenario: Active log reaches its configured maximum size
- **WHEN** appending a record would exceed `logging.maxFileBytes` for a non-empty active log
- **THEN** Tachyon-UI rotates the active file into numbered suffix files
- **AND** it retains no more prior log files than `logging.retainedFiles`

### Requirement: Tachyon-UI records desktop failures visible to the application
Tachyon-UI SHALL record failures from its frontend/backend boundary and
unhandled desktop frontend execution after the logger has initialized.

#### Scenario: A Tauri invocation used by the frontend fails
- **WHEN** a Tachyon-UI frontend operation receives a rejected Tauri invocation
- **THEN** the application submits an `error` log event identified by the failed IPC operation or workflow
- **AND** the existing user feedback or reconnection behavior continues to run

#### Scenario: A frontend exception is not handled by application logic
- **WHEN** the webview raises a global JavaScript error or unhandled promise rejection
- **THEN** Tachyon-UI submits an `error` log event identifying the relevant global frontend source

#### Scenario: The initialized Tauri runtime exits with a fatal error
- **WHEN** Tauri returns a fatal runtime error after the application logger has initialized
- **THEN** Tachyon-UI writes an `error` event from source `application.fatal` before exiting

### Requirement: Desktop error records do not persist IPC input payloads
Tachyon-UI MUST NOT copy frontend IPC invocation arguments into desktop log
events generated for command failure reporting.

#### Scenario: A credential-bearing command fails
- **GIVEN** a frontend invocation contains a password, token, TOTP code, certificate, or configuration payload
- **WHEN** that invocation rejects and Tachyon-UI records the failure
- **THEN** the log event contains the operation source and error message only
- **AND** it does not contain the invocation arguments
