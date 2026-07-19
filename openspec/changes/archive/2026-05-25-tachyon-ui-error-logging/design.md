## Context

Tachyon-UI is a Tauri desktop application with a Rust command layer and a
TypeScript Web Components frontend. Prior to this change, command failures and
frontend exceptions were either shown to the operator or absorbed by retry and
optional-load flows, with no local file available for post-incident diagnosis.

The application already persists security-sensitive state in Stronghold and
passes secrets through IPC operations such as credential saving, signup,
step-up MFA, and certificate configuration. Desktop diagnostics therefore need
to be local and useful while explicitly avoiding the persistence of command
arguments. This concern is distinct from the existing asynchronous guest
stdout/stderr logging capability.

## Goals / Non-Goals

**Goals:**

- Persist desktop startup, fatal, failed IPC, event-listener, and uncaught
  frontend errors in an operator-accessible local journal.
- Provide a configuration file format that can accept future application
  settings in addition to logging configuration.
- Bound disk usage through level filtering and file rotation.
- Preserve the existing UI networking behavior while capturing errors that the
  UI intentionally handles or suppresses.
- Prevent IPC payloads that can contain secrets from being copied into log
  records.

**Non-Goals:**

- Capture guest stdout/stderr or replace the host/system FaaS telemetry
  pipeline.
- Ship local UI logs to a server or expose them through a new mesh API.
- Store full frontend stacks, IPC arguments, response bodies, or Stronghold
  records in the desktop journal.
- Provide an in-app settings screen or hot reload for configuration changes.

## Decisions

### D1 - Use a versioned JSON application configuration in local app data

On startup, the Rust backend loads or creates
`tachyon-ui.config.json` beneath Tauri's `app_local_data_dir()`. The initial
schema is:

```json
{
  "schemaVersion": 1,
  "logging": {
    "level": "info",
    "file": "logs/tachyon-ui.jsonl",
    "maxFileBytes": 5242880,
    "retainedFiles": 5
  }
}
```

JSON is already supported by the crate, is operator-readable, and permits
future top-level settings without changing the storage mechanism. The
`schemaVersion` field permits explicit migrations later. The log path must be
relative to application data, the maximum file size must be at least 1024
bytes, and retained rotated files are limited to 1 through 20.

Alternative considered: TOML. It is suitable for hand editing but would add a
parser dependency and provide no advantage for the current structured settings
or existing JSON tooling.

### D2 - Write newline-delimited JSON locally with bounded rotation

`AppLogger` writes one JSON object per line containing `timestampUnixMs`,
`level`, `source`, and `message`. Severity is filtered using the configured
minimum level (`trace`, `debug`, `info`, `warn`, `error`, or `off`). When an
append would exceed `maxFileBytes`, the writer rotates the file into numbered
suffixes, retaining at most `retainedFiles` prior files.

A small Rust-owned writer avoids adding a logging plugin and keeps the file
schema, validation, and security boundary explicit. Synchronous writes are
acceptable here because the recorded events are desktop control-plane errors,
not request-path workload log streams.

Alternative considered: reuse the guest asynchronous logging system. That
would incorrectly mix UI workstation diagnostics with mesh workload telemetry
and would fail when the UI cannot reach the mesh.

### D3 - Capture frontend failures through a redacted internal Tauri command

The Rust backend exposes `log_frontend_event`, which accepts only log level,
source, and message. `appLogger.ts` invokes this command in a fire-and-forget
path for frontend errors and installs handlers for `window.error` and
`unhandledrejection`.

The existing `network.ts` flow logs in its `catch` paths without replacing its
raw invocation promise chain, preserving component microtask timing and retry
semantics. Production call sites that previously bypassed that network wrapper
use `loggedInvoke`, including MFA, topology, and scope-controller operations.
Listener-registration failures in `main.ts` are also logged.

### D4 - Keep the desktop logging payload free of IPC arguments

Neither `loggedInvoke` nor `network.ts` sends command arguments to
`log_frontend_event`. The log event records the failed command name as a source
label and the returned error message only. Source labels and messages are
bounded in length before persistence.

This protects credentials, enrollment tokens, TOTP codes, certificates, and
configuration payloads carried by commands. It also means that detailed
payload diagnosis must use explicit operator reproduction or separately
approved diagnostic work, rather than silently increasing log sensitivity.

### D5 - Register a backend logger before desktop operations that can fail

During Tauri setup, configuration is loaded, the logger is created and
registered globally, and a startup event is written before Stronghold and
plugin setup proceed. Once registered, a fatal error returned by the Tauri
runtime is appended through the global logger before process exit.

Errors that prevent the configuration file or log writer itself from being
initialized remain visible through the process error path rather than being
recursively written to an unavailable logger.

## Risks / Trade-offs

- **[Risk] A backend-provided error message contains sensitive text**:
  invocation payloads are never logged, but upstream error formatting must
  continue to avoid echoing submitted secrets. -> Mitigation: retain the
  payload-exclusion test and add message-redaction rules if backend contracts
  later expose sensitive error text.
- **[Risk] Local disk failures prevent writing diagnostic records**:
  permission or storage exhaustion can make the journal unavailable. ->
  Mitigation: startup fails clearly if initial logging cannot be established;
  fire-and-forget frontend logging never creates a recursive UI failure loop.
- **[Risk] Very verbose levels consume local disk faster**: `trace` or `debug`
  can generate more records during repeated retries. -> Mitigation:
  size-based rotation and bounded retained-file count limit storage growth.
- **[Trade-off] Configuration is read only on startup**: edits are not
  immediately reflected. -> This avoids introducing a filesystem watcher or a
  mutable runtime configuration service before more settings exist.

## Migration Plan

1. Distribute the updated Tachyon-UI desktop binary and frontend bundle.
2. On first launch after upgrade, create the default JSON configuration and
   `logs/` directory beneath the local application-data directory.
3. Operators who need a different severity or retention policy edit the JSON
   configuration and restart Tachyon-UI.
4. Rollback is non-destructive: an earlier application build can ignore the
   new configuration and JSONL files; operators may remove them separately if
   local diagnostic history is no longer needed.

## Open Questions

- A future configuration change can decide whether to expose log settings in
  the desktop UI; this change intentionally establishes only the file-backed
  configuration contract.
