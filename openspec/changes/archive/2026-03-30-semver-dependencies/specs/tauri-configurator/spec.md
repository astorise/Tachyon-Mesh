## ADDED Requirements

### Requirement: CLI mode seals route SemVer metadata
The `tachyon-cli` application SHALL allow `generate` callers to seal logical route names,
semantic versions, and dependency constraints without opening the desktop UI.

#### Scenario: CLI mode records route identity and dependencies
- **WHEN** a developer invokes `tachyon-cli generate --route /api/faas-a --route-name /api/faas-a=faas-a --route-version /api/faas-a=2.0.0 --route-dependency /api/faas-a=faas-b@^3.1.0 --memory 64`
- **THEN** the generated canonical configuration payload includes `/api/faas-a`
- **AND** the route entry records `name = "faas-a"` and `version = "2.0.0"`
- **AND** the route entry records a dependency map containing `faas-b = "^3.1.0"`
- **AND** the command succeeds without opening a webview

#### Scenario: CLI mode rejects an invalid dependency requirement
- **WHEN** a developer invokes `tachyon-cli generate --route /api/faas-a --route-dependency /api/faas-a=faas-b@not-semver --memory 64`
- **THEN** the command exits with a failure status
- **AND** the error explains that the dependency requirement is not valid SemVer syntax
