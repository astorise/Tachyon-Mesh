# tech-debt Specification Delta

## ADDED Requirements

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

### Requirement: Constrained Decoding Sampler Hygiene
The sampler module SHALL avoid silent integer truncation, unused `PhantomData` markers, and arbitrary `wrapping_*` arithmetic disguised as FSM logic.

#### Scenario: Token id cast preserves width
- **GIVEN** a token id is used inside `CompiledFsm::transition`
- **WHEN** the value is propagated to the next state
- **THEN** the conversion SHALL preserve the original integer width or SHALL be documented as intentionally narrowed

#### Scenario: PhantomData marker is removed
- **GIVEN** the legacy `_sampler_marker: PhantomData<NilSamplerResources>` field exists in `samplers.rs`
- **WHEN** the module is rebuilt
- **THEN** the field SHALL be removed

### Requirement: Strongly Typed Telemetry Registries
The telemetry registries SHALL retrieve metric handles via strongly typed maps or enums rather than runtime `Any::downcast_ref`.

#### Scenario: No Any downcast in registry lookup
- **GIVEN** a host module looks up a telemetry registry
- **WHEN** the code path is inspected
- **THEN** the lookup SHALL NOT rely on `Any::downcast_ref`
- **AND** the lookup SHALL return a strongly typed handle
