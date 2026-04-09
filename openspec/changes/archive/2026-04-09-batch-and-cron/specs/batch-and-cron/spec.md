## ADDED Requirements

### Requirement: Tachyon supports run-to-completion batch targets and scheduled execution
The platform SHALL support command-style WASM targets that run to completion outside the long-lived HTTP server path and SHALL allow scheduled system execution for cron-like maintenance tasks.

#### Scenario: An operator launches a batch target
- **WHEN** the host is invoked to run a batch-oriented target directly or via a cron trigger
- **THEN** it executes the target to completion without starting the standard listeners
- **AND** reports the target exit status back to the caller or scheduler
