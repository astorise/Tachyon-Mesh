## MODIFIED Requirements

### Requirement: Repository packages polyglot WASI guest modules
The repository SHALL include Go, JavaScript, C#, and Java guest examples that compile into standalone WASI modules and ship in the host runtime image alongside the Rust guest modules.

#### Scenario: Container build emits polyglot guest modules
- **WHEN** a developer or CI job runs `docker build -t tachyon-mesh:test .`
- **THEN** the builder stage installs TinyGo, Javy, .NET 8 WASI tooling, and Maven/OpenJDK
- **AND** `guest-go/main.go` is compiled into `guest_go.wasm`
- **AND** `guest-js/index.js` is compiled into `guest_js.wasm`
- **AND** `guest-csharp/Program.cs` is published into `guest_csharp.wasm`
- **AND** `guest-java/src/main/java/com/tachyonmesh/guestjava/Main.java` is compiled into `guest_java.wasm`
- **AND** the runtime image includes all four modules under `/app/guest-modules`

### Requirement: Integration workflow exercises polyglot guest routes
The repository SHALL verify that the deployed host can serve Go, JavaScript, C#, and Java guest modules through sealed HTTP routes without adding a language-specific execution path.

#### Scenario: k3d integration validates polyglot guest responses
- **WHEN** the integration workflow deploys the host image to k3d
- **THEN** the sealed runtime configuration includes `/api/guest-go`, `/api/guest-js`, `/api/guest-csharp`, and `/api/guest-java`
- **AND** `GET /api/guest-go` returns `Hello from TinyGo FaaS!`
- **AND** `GET /api/guest-js` returns `Hello from JavaScript FaaS!`
- **AND** `GET /api/guest-csharp` returns `Hello from C# FaaS!`
- **AND** `GET /api/guest-java` returns `Hello from Java FaaS!`
- **AND** the same host execution pipeline continues to serve the existing Rust guest routes
