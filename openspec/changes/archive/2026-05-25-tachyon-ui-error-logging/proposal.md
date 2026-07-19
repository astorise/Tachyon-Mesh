## Why

Tachyon-UI currently surfaces desktop failures in the interface but does not
persist them for later diagnosis. Operators need a local, configurable error
journal for failures that occur before or during interaction with the mesh,
without leaking authentication material carried by IPC requests.

## What Changes

- Add a desktop application configuration file, generated on first start, with
  an extensible JSON schema and logging settings for severity, relative file
  path, rotation size, and retained file count.
- Add a Rust JSON Lines writer for Tachyon-UI application events, including
  startup and fatal runtime errors, with level filtering and bounded rotation.
- Add an internal Tauri command through which the frontend records failed IPC
  operations, listener registration failures, uncaught JavaScript errors, and
  unhandled promise rejections.
- Route frontend operations that previously invoked Tauri directly through the
  logging boundary, while retaining existing asynchronous behavior in the
  shared network layer.
- Exclude IPC arguments from log events so credentials, TOTP values, tokens,
  certificates, and manifest payloads are not persisted in the error journal.
- Document the generated configuration, log location, accepted levels, and
  restart behavior for operators.

## Capabilities

### New Capabilities

- `tachyon-ui-error-logging`: Local persistent desktop error journaling,
  configurable severity and rotation, frontend-to-backend capture boundaries,
  and secret-safe logging rules for Tachyon-UI.

### Modified Capabilities

None.

## Impact

- `tachyon-ui/src/app_config.rs`: JSON configuration loading, defaults, and
  validation.
- `tachyon-ui/src/app_logging.rs`: structured JSONL writer and log rotation.
- `tachyon-ui/src/main.rs`: logger bootstrap, frontend logging IPC command,
  and fatal error capture.
- `tachyon-ui/src/utils/appLogger.ts`, `tachyon-ui/src/utils/network.ts`, and
  affected component/controller call sites: frontend error capture.
- `tachyon-ui/src/utils/appLogger.test.ts`: verifies failed IPC operations do
  not copy sensitive arguments into journal events.
- `README.md`: operator documentation for configuring and locating desktop
  logs.
- This change does not alter guest/workload asynchronous logging, mesh API
  contracts, or stored Stronghold credential data.
