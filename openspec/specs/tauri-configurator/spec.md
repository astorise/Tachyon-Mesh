# tauri-configurator Specification

## Purpose
TBD - created by archiving change tauri-configurator. Update Purpose after archive.
## Requirements
### Requirement: Tachyon desktop UI is built through a Vite frontend pipeline
The `tachyon-ui` desktop application SHALL build its frontend through a Vite-based toolchain rooted in the `tachyon-ui` directory and SHALL preserve the injected Tailwind CSS and GSAP frontend assets inside that flattened crate layout.

#### Scenario: Desktop frontend assets stay in the `tachyon-ui` crate
- **WHEN** the Tauri desktop application is built
- **THEN** the frontend entry point is `tachyon-ui/index.html`
- **AND** the frontend logic entry point is `tachyon-ui/src/main.ts`
- **AND** the frontend styling entry point is `tachyon-ui/src/style.css`
- **AND** `tachyon-ui/package.json` includes Vite, Tailwind CSS, and GSAP for that frontend bundle

### Requirement: Tauri v2 routes desktop builds through Vite commands
The `tachyon-ui/tauri.conf.json` configuration SHALL use the Tauri v2 build keys required to run the Vite development server and production build pipeline, and SHALL resolve packaged frontend assets from the crate-local `dist` directory.

#### Scenario: Tauri launches the Vite toolchain from the desktop crate
- **WHEN** Tauri reads `tachyon-ui/tauri.conf.json`
- **THEN** `build.beforeDevCommand` is `npm run dev`
- **AND** `build.beforeBuildCommand` is `npm run build`
- **AND** `build.devUrl` is `http://localhost:5173`
- **AND** `build.frontendDist` points to `dist`
- **AND** the resolved frontend asset directory stays inside the `tachyon-ui` crate

### Requirement: The desktop frontend can invoke a Rust status command
The `tachyon-ui` Rust backend SHALL expose a Tauri command named `get_engine_status`, bootstrap directly through `tauri::Builder`, and delegate the status query to `tachyon-client`.

#### Scenario: The frontend requests the engine status through the clean-slate wrapper
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the desktop runtime dispatches the command through `tauri::generate_handler!`
- **AND** the Rust implementation awaits `tachyon_client::get_engine_status()`
- **AND** no CLI-only startup path is evaluated before the desktop window is initialized

### Requirement: The desktop frontend can invoke shared client status queries
The `tachyon-ui` Rust backend SHALL delegate status queries to the shared `tachyon-client` library instead of embedding duplicated lockfile reading logic in the Tauri wrapper.

#### Scenario: The frontend requests the engine status
- **WHEN** the frontend invokes `get_engine_status`
- **THEN** the Tauri command awaits `tachyon_client::get_engine_status()`
- **AND** the returned payload comes from the shared client layer

### Requirement: The desktop wrapper launches without evaluating CLI startup arguments
The `tachyon-ui` project SHALL bootstrap the Tauri runtime immediately on startup and SHALL NOT inspect `std::env::args`, `clap`, or any equivalent CLI parser before the desktop webview is created.

#### Scenario: The GUI binary starts directly in desktop mode
- **WHEN** a user launches `tachyon-ui`
- **THEN** the process enters `tauri::Builder` immediately
- **AND** no manifest-generation or route-parsing code runs before the desktop window is initialized

### Requirement: The desktop wrapper excludes legacy CLI plugin wiring
The `tachyon-ui` project SHALL NOT retain Tauri CLI plugin wiring or desktop config intended for manifest-generation subcommands.

#### Scenario: Tauri config contains no desktop CLI plugin section
- **WHEN** the desktop project configuration is loaded from `tachyon-ui/tauri.conf.json`
- **THEN** the configuration does not declare a `plugins.cli` manifest-generation section
- **AND** the desktop Rust entrypoint does not register `tauri_plugin_cli`

### Requirement: The desktop wrapper keeps a clean-slate Rust dependency surface
The `tachyon-ui` Rust crate SHALL depend only on the shared `tachyon-client` library plus the Tauri runtime and build crates needed for desktop bootstrap, and SHALL NOT pull in legacy CLI or manifest-generation dependencies.

#### Scenario: The Rust crate does not reintroduce CLI-only dependencies
- **WHEN** a developer inspects `tachyon-ui/Cargo.toml`
- **THEN** the runtime dependencies include `tachyon-client` and `tauri`
- **AND** the build dependencies include `tauri-build`
- **AND** the crate does not depend on `clap` or manifest-signing crates

### Requirement: The desktop UI switches management planes without reloading
The `tachyon-ui` frontend SHALL bind sidebar navigation links to pre-rendered management-plane views and switch between them inside the existing `<main>` container without a full page reload.

#### Scenario: The operator selects a different management plane
- **WHEN** the operator clicks a sidebar link for Dashboard, Mesh Topology, Asset Registry, Identity, My Account, or Model Broker
- **THEN** the currently visible panel fades and slides out through GSAP
- **AND** the selected panel fades and slides in within the same page shell
- **AND** the selected sidebar link becomes the active link

### Requirement: The desktop UI exposes dedicated panels for topology, registry, identity, account, and broker workflows
The `tachyon-ui` frontend SHALL expose dedicated panels for mesh topology, asset registry uploads, shared identity posture, personal account security, and AI model brokerage using the shared Tauri commands and widgets already owned by the desktop client.

#### Scenario: The operator opens Mesh Topology
- **WHEN** the Mesh Topology panel becomes active
- **THEN** the frontend invokes `get_mesh_graph`
- **AND** it renders the returned route and batch-target snapshot in the topology view

#### Scenario: The operator opens Asset Registry
- **WHEN** the Asset Registry panel becomes active
- **THEN** the dashboard content is replaced by a panel labeled `Asset Registry`
- **AND** the asset upload controls remain available only in that panel

#### Scenario: The operator opens Identity
- **WHEN** the Identity panel becomes active
- **THEN** the frontend renders the shared administrative user table and connection posture

#### Scenario: The operator opens My Account
- **WHEN** the My Account panel becomes active
- **THEN** the frontend renders personal security actions for the connected operator

#### Scenario: The operator opens Model Broker
- **WHEN** the Model Broker panel becomes active
- **THEN** the frontend renders the chunked model upload controls and progress bar in that panel

### Requirement: The UI Backend MUST strictly validate intents against WIT definitions
The Rust Tauri backend SHALL NOT act as a simple passthrough proxy for JSON payloads. Before dispatching any configuration to the `system-faas-gossip` network, the backend MUST deserialize and validate the JSON payload against the strict Rust structures generated from the `.wit` contracts (e.g., `config-routing.wit`).

#### Scenario: Submitting a malformed route configuration
- **GIVEN** the UI submits a JSON payload for `config-routing` missing the required `bind_address` in a Gateway object
- **WHEN** the Rust backend receives the payload via IPC
- **THEN** the strict Serde deserialization fails
- **AND** the backend returns a safe, handled failure response to the UI
- **AND** the data-plane remains untouched.

### Requirement: Remembered desktop credentials use Stronghold storage
The Tauri backend SHALL store remembered credentials in a Stronghold snapshot and SHALL NOT write passwords or PATs to a plaintext JSON profile.

#### Scenario: Operator stores credentials
- **WHEN** the operator enables remembered credentials
- **THEN** the backend writes the serialized profile to the Stronghold auth record
- **AND** any legacy plaintext profile is migrated or removed

#### Scenario: Stronghold is unavailable
- **WHEN** the operator attempts to enable remembered credentials without a Stronghold backend
- **THEN** the UI disables the toggle
- **AND** it shows an error notification instead of persisting credentials

### Requirement: Step-up MFA uses short-lived host-issued sessions
The desktop step-up MFA flow SHALL use a short-lived session token issued by the host rather than reusing a locally remembered password.

#### Scenario: Operator completes step-up MFA
- **WHEN** the operator submits a valid six-digit MFA code during a sensitive action
- **THEN** the Tauri backend requests a host step-up session token
- **AND** the local password profile is not read or replayed

### Requirement: Runtime configuration panels mutate the manifest directly
Configuration panels that affect `core-host` runtime behavior SHALL read the active manifest with `get_manifest_config`, mutate the corresponding `IntegrityConfig` field, and apply the result with `apply_manifest_config`. They SHALL NOT rely on `ui_configurations` overlays for runtime configuration.

#### Scenario: Operator applies a runtime-backed configuration
- **WHEN** a configuration panel changes a runtime field such as `routes[].canary`, `routes[].scopes`, or `kv_caches`
- **THEN** the frontend reads the current manifest
- **AND** mutates only the targeted manifest field
- **AND** submits the complete updated config via `apply_manifest_config`
- **AND** the applied manifest is visible to `core-host`

#### Scenario: Operator edits remaining IntegrityConfig controls
- **WHEN** the operator edits enrollment policy, TEE backend, layer4 bindings, scope enforcement, trusted signers, telemetry sample rate, instance-pool memory, cloud sync endpoint, batch targets, or advanced route volume policy
- **THEN** Tachyon Studio reads the active manifest with `get_manifest_config`
- **AND** writes the corresponding snake_case `IntegrityConfig` field
- **AND** applies the signed full manifest with `apply_manifest_config`
- **AND** does not stage a `ui_configurations` overlay for those runtime fields

#### Scenario: Operator toggles route TEE execution
- **WHEN** the operator toggles a route's TEE state from the routing table
- **THEN** Tachyon Studio mutates only `routes[].requires_tee` for the selected route
- **AND** keeps `IntegrityConfig.tee_backend` as the global backend selector
- **AND** applies the full manifest through `apply_manifest_config`

#### Scenario: Operator configures zero-touch enrollment from Nodes
- **WHEN** the operator sets `enrollment.mode` to `zero-touch` or `both`
- **THEN** Tachyon Studio requires `enrollment.oidc_issuer`
- **AND** accepts optional `enrollment.oidc_audience`
- **AND** validates `enrollment.auto_approve_tags` as `key=value` matchers before applying the manifest

#### Scenario: Operator edits trusted manifest signers from Identity
- **WHEN** the operator saves trusted signers
- **THEN** Tachyon Studio writes `trusted_signers` as newline or comma separated 64-character hex Ed25519 public keys
- **AND** rejects malformed public keys before applying the manifest

#### Scenario: Operator configures advanced volume policy
- **WHEN** the operator edits a route volume from the expanded Routing view
- **THEN** Tachyon Studio can write `type`, `encrypted`, `ttl_seconds`, `idle_timeout`, `eviction_policy`, `backup_schedule`, and `consistency`
- **AND** the update is scoped to the matching `routes[].volumes[]` entry

#### Scenario: Legacy domain payload cannot be staged as a fake runtime change
- **WHEN** a configuration panel submits a validated domain payload through `apply_configuration`
- **THEN** the Tauri backend returns a handled failure explaining that `ui_configurations` overlays are ignored by `core-host`
- **AND** no payload is staged into the local configuration overlay
- **AND** the UI must migrate that panel to a manifest-backed controller before claiming runtime application

### Requirement: Additional config WIT bindings are wired
The Tauri backend SHALL generate and reference WIT bindings for routing plus the additional AI, resilience, observability, storage, and fleet configuration contracts.

#### Scenario: A panel validates a configured domain
- **WHEN** a supported panel submits a domain payload
- **THEN** the backend uses generated WIT contract types in the validation path
- **AND** unsupported domains are rejected explicitly

### Requirement: Strict Content Security Policy
The Tauri WebView SHALL enforce a strict Content Security Policy that blocks inline scripts and evals while permitting Tauri IPC, local WebSockets, and data URI images.

#### Scenario: CSP blocks inline script injection
- **GIVEN** the Tauri application is running
- **WHEN** a script is injected inline via a compromised DOM node
- **THEN** the CSP `script-src 'self'` directive blocks its execution

#### Scenario: QR code images are rendered without innerHTML
- **GIVEN** a TOTP enrollment or operator invite flow displays a QR code
- **WHEN** the QRCode library generates the code
- **THEN** it is rendered as a PNG data URI set on an `<img>` `src` attribute
- **AND** no SVG HTML is inserted via `innerHTML`

### Requirement: DOM manipulation safety
All dynamic data inserted into the DOM via `innerHTML` in the Tachyon UI MUST either be escaped through `escapeHtml()` or replaced with safe DOM APIs (`textContent`, `replaceChildren`).

#### Scenario: Nav link labels are HTML-escaped
- **GIVEN** the AppShell renders navigation links from the ComponentRegistry
- **WHEN** an entry label or route string contains HTML metacharacters
- **THEN** those characters are escaped before insertion into `innerHTML`

### Requirement: Hardware panel MUST display per-GPU VRAM progress bars
`TachyonHardwarePanel` SHALL fetch both `get_hardware_status` and `get_metrics` on mount and render a VRAM section with one progress bar per detected GPU (or a single cluster-wide bar from `vramUtilizationPct`). The bar colour SHALL be cyan < 80 %, amber 80–90 %, red ≥ 90 %.

#### Scenario: VRAM section appears when metrics are available
- **GIVEN** `get_metrics` returns `vramUtilizationPct: 85`
- **WHEN** the hardware panel mounts
- **THEN** an amber progress bar at 85 % width is rendered inside the VRAM section

#### Scenario: RAM offload badge appears when active
- **GIVEN** `get_metrics` returns `ramOffloadActive: true`
- **WHEN** the hardware panel renders
- **THEN** the offload badge is visible in the VRAM section

### Requirement: Storage panel MUST include a KV-Partition V2 Explorer
`TachyonStoragePanel` SHALL render a KV Explorer section with namespace and key inputs, a Get button calling `invoke("kv_get")`, a Delete button calling `invoke("kv_delete")`, and an inline result zone. Values that parse as JSON SHALL be pretty-printed. Both buttons SHALL be disabled during in-flight requests.

#### Scenario: Get returns and displays the stored value
- **GIVEN** a key `sessions/user:alice:prefs` exists in the KV store
- **WHEN** the operator enters the namespace and key and clicks Get
- **THEN** the result zone renders the value pretty-printed

#### Scenario: Delete dispatches a success toast
- **GIVEN** a key exists in the KV store
- **WHEN** the operator clicks Delete
- **THEN** a `toast` CustomEvent with `type: "success"` is dispatched on the window
