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

