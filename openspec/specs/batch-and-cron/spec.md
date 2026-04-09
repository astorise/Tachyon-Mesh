# Batch And Cron

## Purpose
Define Tachyon's run-to-completion execution path for command-style WASM components and the sealed manifest shape used to register batch targets such as maintenance jobs.
## Requirements
### Requirement: The host supports a command-style batch execution mode
The host SHALL expose a `run` subcommand that loads a signed manifest, resolves a named batch target, executes it without starting long-lived listeners, and maps the guest result to the host process exit code.

#### Scenario: An operator launches a batch target
- **WHEN** `core-host run --manifest <path> --target <name>` is invoked
- **THEN** the host resolves the named batch target from the sealed manifest
- **AND** instantiates the target as a WASI command component
- **AND** exits with code `0` on success or `1` on failure

### Requirement: Batch targets can declare environment variables and volume mounts
The sealed manifest SHALL allow batch targets to declare command modules, environment variables, and guest-visible volume mounts independently from HTTP routes.

#### Scenario: A batch target requires a writable cache directory
- **WHEN** a manifest batch target declares `env` entries and a mounted host path
- **THEN** the host injects those environment variables into the WASI context
- **AND** preopens the declared guest path before invoking `wasi:cli/run`

### Requirement: The GC system FaaS runs as a command component
The `system-faas-gc` guest SHALL run as a command-style WASM module that recursively deletes stale files from a mounted directory according to a TTL expressed in seconds.

#### Scenario: The GC job sweeps a stale file
- **WHEN** the GC batch target runs with `TTL_SECONDS` and `TARGET_DIR` configured
- **THEN** it prints deletion logs for expired files
- **AND** removes the expired files from the mounted host directory

### Requirement: Tachyon supports run-to-completion batch targets and scheduled execution
The platform SHALL support command-style WASM targets that run to completion outside the long-lived HTTP server path and SHALL allow scheduled system execution for cron-like maintenance tasks.

#### Scenario: An operator launches a batch target
- **WHEN** the host is invoked to run a batch-oriented target directly or via a cron trigger
- **THEN** it executes the target to completion without starting the standard listeners
- **AND** reports the target exit status back to the caller or scheduler

