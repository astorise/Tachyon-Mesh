## ADDED Requirements

### Requirement: Host supports command-style WASI guest entrypoints
The `core-host` runtime SHALL execute guest modules that export either the existing `faas_entry` function or the standard WASI command entrypoint `_start`, while preserving the same stdin/stdout contract for both.

#### Scenario: Guest module exposes `_start` instead of `faas_entry`
- **WHEN** the host loads a guest module that does not export `faas_entry` but does export `_start`
- **THEN** the host invokes `_start`
- **AND** the guest still receives the HTTP request body through WASI stdin
- **AND** the host still returns the captured WASI stdout as the HTTP response body
