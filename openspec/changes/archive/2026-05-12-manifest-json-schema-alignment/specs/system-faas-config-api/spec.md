# Technical Specification: WASM-Based Validation

## 1. Structured Error Types
Define the standard response format in `systems/system-faas-config-api/src/lib.rs` (and share it with `tachyon-client`).

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

## 2. Validation Logic implementation
Inside `system-faas-config-api`, add a JSON Schema validation crate (like `jsonschema`). 
When a dry-run is invoked:
1. Fetch the official schema (either passed down from host context or fetched via allowed HTTP egress to `localhost/admin/schema`).
2. Validate the incoming manifest payload.
3. Map the schema validation errors into `Vec<ValidationError>`.
4. Return the serialized `DryRunResult`.