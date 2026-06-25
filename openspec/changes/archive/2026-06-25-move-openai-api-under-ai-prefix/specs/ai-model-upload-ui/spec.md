## MODIFIED Requirements

### Requirement: Upload result and registry discoverability

On success the panel SHALL display the returned asset reference and indicate that the model will appear in the model list (`GET /ai/v1/models`); on failure it SHALL display the translated backend error.

#### Scenario: Successful upload shows asset ref and registry hint

- **WHEN** `push_large_model` resolves successfully
- **THEN** the panel shows the returned asset ref
- **AND** the panel indicates the model is being registered and will appear in the model list (`/ai/v1/models` via `guest-openai`)

#### Scenario: Failed upload shows a translated error

- **WHEN** `push_large_model` rejects with a backend error
- **THEN** the panel shows the translated error message and re-enables the upload control
