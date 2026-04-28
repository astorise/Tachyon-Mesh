# large-model-broker Specification

## Purpose
TBD - created by archiving change large-model-broker. Update Purpose after archive.
## Requirements
### Requirement: Large model uploads are streamed directly to disk
The host SHALL stream large model uploads into staging files on disk instead of buffering the complete payload in memory.

#### Scenario: A multipart upload is in progress
- **WHEN** the client uploads chunk `N` for an initialized model upload
- **THEN** the host appends the raw bytes directly to the upload staging file
- **AND** it tracks the received byte count and expected part order

### Requirement: Completed uploads are verified before finalization
The host SHALL hash the staged file during commit and only finalize it when the hash and size match the initialized metadata.

#### Scenario: A model upload commits successfully
- **WHEN** the staged file hash matches the expected `sha256:<digest>` and the received size matches
- **THEN** the host moves the file into `tachyon_data/models`
- **AND** it returns the finalized model path

#### Scenario: A model upload fails verification
- **WHEN** the staged file hash or size differs from the initialized metadata
- **THEN** the host rejects the commit
- **AND** it removes the staging file

### Requirement: The desktop UI reports multipart progress
The desktop UI SHALL show upload progress for large-model uploads.

#### Scenario: A model is being streamed
- **WHEN** the Tauri command emits `upload_progress`
- **THEN** the UI updates the progress bar width to reflect the current percentage

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

