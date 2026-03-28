# polyglot-faas Specification

## Purpose
TBD - created by archiving change polyglot-faas. Update Purpose after archive.
## Requirements
### Requirement: Repository packages polyglot WASI guest modules
The repository SHALL include Go and JavaScript guest examples that compile into standalone WASI modules and ship in the host runtime image alongside the Rust guest modules.

#### Scenario: Container build emits Go and JavaScript guest modules
- **WHEN** a developer or CI job runs `docker build -t tachyon-mesh:test .`
- **THEN** the builder stage installs TinyGo and Javy
- **AND** `guest-go/main.go` is compiled into `guest_go.wasm`
- **AND** `guest-js/index.js` is compiled into `guest_js.wasm`
- **AND** the runtime image includes both modules under `/app/guest-modules`

### Requirement: Integration workflow exercises polyglot guest routes
The repository SHALL verify that the deployed host can serve Go and JavaScript guest modules through sealed HTTP routes without adding a language-specific execution path.

#### Scenario: k3d integration validates polyglot guest responses
- **WHEN** the integration workflow deploys the host image to k3d
- **THEN** the sealed runtime configuration includes `/api/guest-go` and `/api/guest-js`
- **AND** `GET /api/guest-go` returns `Hello from TinyGo FaaS!`
- **AND** `GET /api/guest-js` returns `Hello from JavaScript FaaS!`
- **AND** the same host execution pipeline continues to serve the existing Rust guest routes

