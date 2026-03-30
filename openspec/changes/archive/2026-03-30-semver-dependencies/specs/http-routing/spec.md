## ADDED Requirements

### Requirement: Host validates sealed mesh dependencies before serving traffic
The `core-host` runtime SHALL build a registry of sealed routes keyed by logical service name,
validate every declared dependency requirement against that registry at startup, and refuse to
serve traffic when no compatible version is loaded.

#### Scenario: Startup fails when a compatible dependency version is missing
- **WHEN** route `faas-a@2.0.0` declares `faas-b = "^2.0"`
- **AND** the sealed configuration only loads `faas-b@1.5.0`
- **THEN** `core-host` aborts startup before binding the HTTP listener
- **AND** the error explains that no compatible `faas-b` version was loaded

### Requirement: Host resolves internal mesh aliases with SemVer-aware routing
The `core-host` runtime SHALL resolve internal mesh URLs like `http://tachyon/<service>` or
`http://mesh/<service>` by consulting the caller's sealed dependency constraints and selecting the
highest compatible loaded route version for that logical service.

#### Scenario: Highest compatible route version is selected
- **WHEN** route `faas-a@2.0.0` declares `faas-b = "^2.0"`
- **AND** the sealed configuration loads `faas-b@2.1.0` at `/api/faas-b-v2`
- **AND** the sealed configuration also loads `faas-b@3.0.0` at `/api/faas-b-v3`
- **AND** `faas-a` emits `MESH_FETCH:http://tachyon/faas-b`
- **THEN** the host rewrites the internal request to `/api/faas-b-v2`
- **AND** the breaking `3.0.0` route is ignored for that call

#### Scenario: Undeclared internal dependency is rejected
- **WHEN** a route emits `MESH_FETCH:http://tachyon/faas-b`
- **AND** its sealed dependency map does not declare `faas-b`
- **THEN** the host rejects the mesh fetch
- **AND** the response surfaces a dependency-declaration error
