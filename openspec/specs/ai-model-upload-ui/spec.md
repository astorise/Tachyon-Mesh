# ai-model-upload-ui Specification

## Purpose
TBD - created by archiving change ai-model-upload-ui. Update Purpose after archive.
## Requirements
### Requirement: Operator can upload a model file from the UI
The Tachyon UI SHALL provide a `<tachyon-model-upload-panel>` control, mounted in the AI Orchestration view, that lets an operator select a local model file and upload it to the cluster by invoking the `push_large_model` command with the selected file path.

#### Scenario: Operator selects a file and starts an upload
- **WHEN** the operator opens the native file picker from the panel and selects a model file
- **THEN** the panel obtains the file's filesystem path and invokes `push_large_model` with that path

#### Scenario: HTML file input is not used for path resolution
- **WHEN** the panel needs a filesystem path for the upload
- **THEN** it obtains it from the native file-open dialog, not from an HTML `<input type="file">` (which does not expose a real path under Tauri)

### Requirement: Model upload is step-up gated
The panel SHALL invoke `push_large_model` through the privileged-command wrapper so that step-up MFA is enforced; it SHALL NOT call the Tauri command directly bypassing that wrapper.

#### Scenario: Upload requires step-up
- **WHEN** the operator triggers a model upload
- **THEN** the privileged-command wrapper enforces step-up authentication before the upload proceeds (because `push_large_model` is a step-up command)

### Requirement: Live upload progress
The panel SHALL subscribe to the `upload_progress` event for the duration of an upload and render the reported percentage, and SHALL disable the upload control while an upload is in progress.

#### Scenario: Progress bar reflects upload percentage
- **WHEN** an upload is running and the backend emits `upload_progress` with a percentage
- **THEN** the panel updates a visible progress indicator to that percentage
- **AND** the upload control is disabled until the upload completes or fails

### Requirement: Upload result and registry discoverability

On success the panel SHALL display the returned asset reference and indicate that the model will appear in the model list (`GET /ai/v1/models`); on failure it SHALL display the translated backend error.

#### Scenario: Successful upload shows asset ref and registry hint

- **WHEN** `push_large_model` resolves successfully
- **THEN** the panel shows the returned asset ref
- **AND** the panel indicates the model is being registered and will appear in the model list (`/ai/v1/models` via `guest-openai`)

#### Scenario: Failed upload shows a translated error

- **WHEN** `push_large_model` rejects with a backend error
- **THEN** the panel shows the translated error message and re-enables the upload control

### Requirement: Upload panel gating
The model-upload panel SHALL be rendered only within the AI Orchestration view, which is shown when `has_ai` is true. Model upload depends on an active `model-broker` (an `ai-inference` build); when the broker is absent the upload SHALL surface the broker's backend error through the normal error path.

#### Scenario: Panel hidden without AI capability
- **WHEN** `has_ai` is false for the connected cluster
- **THEN** the AI Orchestration view (and therefore the upload panel) is not shown

