# Implementation Tasks

## Phase 1: Struct and Deserialization Hardening (`core-host`)
- [x] Locate the configuration payload structures in `core-host/src/` (likely related to routing and integrity).
- [x] Remove `Option` wrappers and `#[serde(default)]` macros from `version` and `dependencies` fields.
- [x] Update the JSON deserialization logic to strictly enforce the presence of these fields.

## Phase 2: Boot Sequence and Error Handling (`core-host`)
- [x] Update the startup sequence so that a deserialization error of the `integrity.lock` triggers a fatal log `ERR_INTEGRITY_SCHEMA_VIOLATION`.
- [x] Ensure the process exits with a non-zero code immediately, before any network binding occurs.

## Phase 3: Manifest Production Realignment
- [x] Ensure canonical route serialization always writes a `version` field and a `dependencies` object, even when dependencies are empty.
- [x] Regenerate the checked-in signed `integrity.lock` so the embedded `core-host` payload satisfies the strict schema.

## Phase 4: Test Realignment
- [x] Update `openspec/specs/cryptographic-integrity/spec.md` to remove the "Fallback / Older manifest" scenarios.
- [x] Fix any broken unit/integration tests that were relying on generating legacy `integrity.lock` files.
- [x] Add a new unit test in `core-host` verifying that a payload missing a `version` field triggers the expected panic.
