## ADDED Requirements

### Requirement: Model broker writes large downloads to a .part file and renames atomically
`system-faas-model-broker` SHALL stream large model downloads (e.g. GGUF) into a temporary file with a `.part` suffix, and SHALL rename the file to its final name only after the entire stream completes successfully.

#### Scenario: Successful download is renamed atomically
- **WHEN** a model download stream completes successfully
- **THEN** the broker performs an `fs::rename` from `<file>.part` to `<file>`
- **AND** any consumer reading the directory observes either the absent file or the fully written final file
- **AND** no consumer ever observes a partially written file under the final name

### Requirement: Aborted downloads do not leak partial files
If a download stream is interrupted (client abort, network error, host shutdown), the broker SHALL immediately attempt to remove the `.part` file, and any orphaned `.part` files left after a hard crash SHALL be eligible for cleanup by `system-faas-gc` based on its TTL configuration.

#### Scenario: Aborted upload is cleaned up
- **WHEN** a download stream errors or the client disconnects mid-stream
- **THEN** the broker removes the corresponding `.part` file
- **AND** the final file name does not appear on disk
- **AND** the AI inference engine never attempts to load that partial file

#### Scenario: Orphaned .part file is reaped after a crash
- **WHEN** an `.part` file remains on disk after a host crash
- **AND** the file's age exceeds the configured GC TTL
- **THEN** `system-faas-gc` removes the orphaned `.part` file during a sweep
