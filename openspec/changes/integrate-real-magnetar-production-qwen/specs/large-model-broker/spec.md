## ADDED Requirements

### Requirement: Model broker treats uploaded model artifacts as format-neutral
`system-faas-model-broker` SHALL verify upload manifests, install artifact directories, write host-controlled provenance metadata, and publish a Magnetar-owned model upload event without deciding whether the bytes are GGUF, Safetensors, or another model format.

#### Scenario: Upload commit does not infer model format
- **WHEN** a model archive is committed successfully
- **THEN** the broker unpacks and verifies the declared files
- **AND** the broker writes alias and upload provenance metadata
- **AND** it does not write a model `format` declaration into the provenance sidecar

#### Scenario: Magnetar owns model format support
- **WHEN** the broker publishes the model upload event
- **THEN** the event identifies the installed path as a Magnetar artifact
- **AND** format validation remains the responsibility of Magnetar production ingestion

## MODIFIED Requirements

### Requirement: Model broker writes large downloads to a .part file and renames atomically
`system-faas-model-broker` SHALL stream large model artifact downloads into a temporary file with a `.part` suffix, and SHALL rename the file to its final name only after the entire stream completes successfully.

#### Scenario: Successful download is renamed atomically
- **WHEN** a model download stream completes successfully
- **THEN** the broker performs an `fs::rename` from `<file>.part` to `<file>`
- **AND** any consumer reading the directory observes either the absent file or the fully written final file
- **AND** no consumer ever observes a partially written file under the final name
