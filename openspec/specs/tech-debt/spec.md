# tech-debt Specification

## Purpose
TBD - created by archiving change v1-1-audit-p2-techdebt. Update Purpose after archive.
## Requirements
### Requirement: Host File Range Path Canonicalization
The host file-piping utility (`pipe_range_from_file` or equivalent) SHALL canonicalize the requested path and SHALL reject any path whose canonical form does not start with the allowed root directory.

#### Scenario: Path traversal attempt is rejected
- **GIVEN** a guest requests range read of `../../etc/passwd`
- **WHEN** the host resolves the path
- **THEN** the host SHALL refuse the request with an error
- **AND** no file descriptor SHALL be opened outside the allowed root

#### Scenario: Allowed paths inside the root are accepted
- **GIVEN** a guest requests range read of a file within the configured asset root
- **WHEN** the host canonicalizes the path
- **THEN** the file SHALL be opened and the requested byte range SHALL be streamed

### Requirement: Safe Safetensors Memory Mapping
`core-host/src/ai_inference.rs` SHALL NOT use `unsafe` array coercion when mapping Safetensors slices. The mapping logic SHALL either use safe byte-slice conversion or SHALL return `unimplemented!()` while the feature remains experimental.

#### Scenario: No unsafe coercion in safetensors mapping
- **GIVEN** a Safetensors file is mapped via `LayerWiseMappedModel`
- **WHEN** the source is compiled with default features
- **THEN** the file SHALL NOT contain an `unsafe` block in the safetensors flat-array coercion path

### Requirement: Removed Sampler Runtime Stays Absent
The removed in-process sampler runtime SHALL stay absent from active local inference code unless it is reintroduced through a new accepted design and implementation.

#### Scenario: No orphan sampler module remains
- **GIVEN** the post-Magnetar-cutover source tree
- **WHEN** active local inference files are inspected
- **THEN** no standalone sampler runtime module is compiled

#### Scenario: Future sampler work requires an explicit contract
- **GIVEN** a future change needs provider-side sampling behavior
- **WHEN** the change is proposed
- **THEN** it SHALL define the Provider/runtime contract instead of restoring an implicit local fallback

### Requirement: Strongly Typed Telemetry Registries
The telemetry registries SHALL retrieve metric handles via strongly typed maps or enums rather than runtime `Any::downcast_ref`.

#### Scenario: No Any downcast in registry lookup
- **GIVEN** a host module looks up a telemetry registry
- **WHEN** the code path is inspected
- **THEN** the lookup SHALL NOT rely on `Any::downcast_ref`
- **AND** the lookup SHALL return a strongly typed handle

