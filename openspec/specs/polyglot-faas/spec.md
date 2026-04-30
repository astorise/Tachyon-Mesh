# polyglot-faas Specification

## Purpose
Define polyglot Wasm component execution and official SDK generation for Tachyon FaaS guests.

## Requirements
### Requirement: WIT-driven guest interface
Official SDKs SHALL be generated from the canonical Tachyon WIT interface so every supported language targets the same binary contract.

#### Scenario: WIT interface changes
- **WHEN** the canonical WIT interface changes
- **THEN** generated SDK bindings for supported languages are refreshed from the same source interface

### Requirement: Official SDK publishing
The release pipeline SHALL publish generated SDKs to the configured ecosystem registries.

#### Scenario: SDK publish workflow runs
- **WHEN** a release triggers SDK publishing
- **THEN** Rust, JavaScript or TypeScript, Python, and Go SDK outputs are generated and published through their configured channels
