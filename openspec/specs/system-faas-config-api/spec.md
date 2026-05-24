# system-faas-config-api Specification

## Purpose
WASM-based configuration validation and GitOps brokering for the Tachyon Mesh system-faas layer.

## Requirements

### Requirement: Structured validation responses
`system-faas-config-api` SHALL define the standard dry-run response format in `systems/system-faas-config-api/src/lib.rs` and share compatible types with `tachyon-client`.

```rust
#[derive(Serialize, Deserialize, Debug)]
pub struct ValidationError {
    pub path: String,       // e.g., "spec.functions[0].minRamMb"
    pub message: String,
    pub error_code: String, // e.g., "INVALID_TYPE" or "MISSING_FIELD"
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DryRunResult {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub diff: Option<serde_json::Value>,
}
```

#### Scenario: Dry run returns structured validation errors
- **GIVEN** a submitted manifest violates the official schema
- **WHEN** `system-faas-config-api` performs a dry run
- **THEN** it returns `valid: false`
- **AND** each validation error includes `path`, `message`, and `error_code`

### Requirement: WASM-based schema validation
`system-faas-config-api` SHALL validate dry-run manifest payloads against the official JSON Schema and serialize the result as `DryRunResult`.

#### Scenario: Manifest is validated against official schema
- **WHEN** a dry-run request is invoked
- **THEN** the FaaS fetches or receives the official manifest schema
- **AND** validates the incoming manifest payload against that schema
- **AND** maps schema validation failures into `Vec<ValidationError>`
