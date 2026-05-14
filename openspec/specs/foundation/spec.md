# foundation Specification

## Purpose
TBD - created by archiving change tachyon-ui-config-foundation. Update Purpose after archive.
## Requirements
### Requirement: Configuration dashboards MUST share a base Web Component class
Tachyon UI SHALL provide a `TachyonConfigDashboard` base class extending `HTMLElement` for configuration-domain dashboards.

#### Scenario: Dashboard subclasses inherit Shadow DOM setup
- **GIVEN** a configuration dashboard extends `TachyonConfigDashboard`
- **WHEN** the dashboard custom element is constructed
- **THEN** the base class attaches an open Shadow DOM
- **AND** the subclass can render its template without querying the global document.

#### Scenario: Dashboard subclasses render through the base template helper
- **GIVEN** a dashboard subclass provides HTML content
- **WHEN** it calls the base `renderTemplate` helper
- **THEN** the content is rendered into the component Shadow DOM
- **AND** existing global app-shell DOM is not overwritten.

### Requirement: Configuration dashboards MUST use a shared constructable stylesheet
Tachyon UI SHALL expose a shared `CSSStyleSheet` derived from the project Tailwind output for dashboard Web Components.

#### Scenario: Dashboard applies shared Tachyon styling
- **GIVEN** a dashboard extends `TachyonConfigDashboard`
- **WHEN** the dashboard is connected to the document
- **THEN** the base class applies the shared stylesheet to `shadowRoot.adoptedStyleSheets`
- **AND** the dashboard uses the Dark Slate/Cyan visual language without duplicating style text per instance.

### Requirement: Configuration dashboards MUST provide zero-panic feedback rendering
Tachyon UI SHALL provide a standardized `showFeedback(type, message)` helper for success and error feedback inside dashboard Shadow DOMs.

#### Scenario: Rust command failure is displayed inline
- **GIVEN** a dashboard receives a handled Rust or Tauri command failure
- **WHEN** it calls `showFeedback("error", message)`
- **THEN** the dashboard displays an error-styled feedback block inside `#feedback-zone`
- **AND** the application shell remains mounted.

#### Scenario: Successful configuration is displayed inline
- **GIVEN** a dashboard receives a successful configuration response
- **WHEN** it calls `showFeedback("success", message)`
- **THEN** the dashboard displays a success-styled feedback block inside `#feedback-zone`
- **AND** the feedback entrance is animated without blocking input.

### Requirement: App Shell MUST resolve configuration views through a component registry
Tachyon UI SHALL provide a component registry that maps sidebar route slugs to custom element tags for configuration dashboards.

#### Scenario: Sidebar route mounts a registered component
- **GIVEN** a route slug is registered with a dashboard custom element tag
- **WHEN** the operator selects that route in the App Shell sidebar
- **THEN** the App Shell mounts the registered component in `#router-view`
- **AND** unknown route slugs are handled without throwing.

### Requirement: Skeleton loading states
Every domain panel that performs a remote data fetch SHALL display a shimmer skeleton while the fetch is pending.

#### Scenario: Panel displays skeleton during data load
- **GIVEN** a domain panel is mounted and its async fetch has not yet resolved
- **WHEN** the panel's `connectedCallback` calls `withLoadingState`
- **THEN** the panel content area shows `.skeleton-pulse` shimmer blocks

#### Scenario: Fetch failure triggers actionable toast
- **GIVEN** `withLoadingState` task throws
- **WHEN** `handlePanelError` is called with the error and the retry task
- **THEN** a toast is dispatched with `type: "error"` and an inline "Retry" button
- **AND** clicking Retry re-invokes `withLoadingState` from the beginning

### Requirement: Actionable error toasts
`TachyonToastManager` SHALL render an inline action button when `ToastDetail.action` is provided.

#### Scenario: Toast with action button
- **GIVEN** a `"toast"` event is dispatched with `action: { label, onClick }`
- **WHEN** `TachyonToastManager` processes the event
- **THEN** the toast element includes a button labeled with `action.label`
- **AND** clicking the button invokes `action.onClick` then dismisses the toast
- **AND** the toast remains visible for 8 seconds instead of the default 4

### Requirement: Keyboard accessibility — focus visibility
Every interactive element (button, link, input, select) SHALL display a visible `:focus-visible` outline.

#### Scenario: Keyboard user reaches a button
- **WHEN** a user navigates to any interactive element via keyboard
- **THEN** a 2px blue ring appears around the element with a slate-900 offset

### Requirement: Semantic layout and skip navigation
The app shell SHALL expose landmark roles and a skip-navigation link.

#### Scenario: Screen reader user lands on the application
- **WHEN** the application shell renders
- **THEN** `<aside>`, `<nav>`, `<header>`, and `<main>` elements are present with `aria-label` attributes
- **AND** a skip link targeting `#main-content` is the first focusable element
- **AND** `<main id="main-content" tabindex="-1">` accepts programmatic focus

### Requirement: Labelled form inputs
Every form `<input>` SHALL have an associated label visible to screen readers.

#### Scenario: Screen reader user fills the login form
- **GIVEN** TachyonIAM renders in login mode
- **WHEN** the user navigates to any input field
- **THEN** the input has an associated `<label class="sr-only">` and `aria-required="true"`

### Requirement: ARIA live region for connection status
The NetworkStatus component SHALL announce connection state changes via an ARIA live region.

#### Scenario: Connection drops
- **WHEN** the network status changes to "Disconnected"
- **THEN** the `role="status"` / `aria-live="polite"` container announces the change without interrupting the user

### Requirement: Modal dialog accessibility
Overlay dialogs SHALL carry `role="dialog"`, `aria-modal="true"`, a labelled heading, and a keyboard focus trap.

#### Scenario: Conflict modal opens
- **WHEN** TachyonBundleConflictModal renders with conflicts
- **THEN** the backdrop has `role="dialog"` and `aria-modal="true"`
- **AND** focus is moved inside the modal
- **AND** Tab/Shift+Tab cycles only through the modal's focusable elements

### Requirement: AppShell navigation MUST be a standalone Web Component
The sidebar navigation SHALL be extracted into `TachyonAppShellNav` (`tachyon-app-shell-nav`). It SHALL observe the `active-route` attribute and emit `shell:navigate` (bubbles, composed) with `{ route }` on link clicks.

#### Scenario: Active route attribute drives highlight
- **GIVEN** `<tachyon-app-shell-nav active-route="topology">` is rendered
- **WHEN** `attributeChangedCallback` fires
- **THEN** the topology button has the active CSS classes and all other buttons do not

#### Scenario: Link click dispatches navigation event
- **WHEN** an operator clicks a sidebar link
- **THEN** a `shell:navigate` CustomEvent bubbles with `detail.route` matching the link's `data-route`
- **AND** `window.location.hash` is updated to the same route

### Requirement: A reusable focus trap utility MUST exist for modal dialogs
`tachyon-ui/src/utils/a11y.ts` SHALL export `trapFocus(element): () => void` that cycles Tab/Shift+Tab focus within the container, moves focus to the first focusable child on call, and returns a cleanup function.

### Requirement: IAM dialog MUST carry ARIA dialog role and focus trap
`TachyonIAM.ts` SHALL render `#iam-panel` with `role="dialog"`, `aria-modal="true"`, and `aria-labelledby="iam-dialog-title"`. `trapFocus` SHALL be called on the panel immediately after rendering.

### Requirement: Bundle conflict modal MUST carry ARIA dialog role and focus trap
`TachyonAppShellModalRoot.openConflictModal()` SHALL set `role="dialog"` and `aria-modal="true"` on the conflict modal element and call `trapFocus` after opening it.

### Requirement: Seal-and-apply operation MUST show an accessible global loader
During `sealAndApply()`, `TachyonAppShell` SHALL set `aria-busy="true"` on `#main-content`, overlay a spinner with `role="status"`, and remove both in the `finally` block.

### Requirement: A zero-build installer script MUST exist for operators
The repository SHALL provide `scripts/get-tachyon.sh` that downloads pre-compiled `core-host` and `tachyon-mcp` binaries from the latest GitHub release without requiring a Rust toolchain. It SHALL accept `--version` and `--dir` flags, detect OS and architecture, and print a success banner with the MCP config snippet. It SHALL exit 1 with a build-from-source hint when the download fails.

#### Scenario: Operator downloads latest release
- **GIVEN** a GitHub release exists for the repository
- **WHEN** `curl -fsSL .../get-tachyon.sh | bash` is run on a supported platform
- **THEN** `core-host` and `tachyon-mcp` are extracted to the current directory
- **AND** the script exits 0 and prints the binary paths

#### Scenario: No release exists — graceful failure
- **GIVEN** no GitHub release exists (pre-launch)
- **WHEN** the download script is run
- **THEN** the script exits 1 with a message directing the user to `./scripts/setup.sh`

### Requirement: A K8s operator E2E workflow MUST test the local-image deployment path
`.github/workflows/e2e-k8s.yml` SHALL build a local Docker image, import it into a k3d cluster, patch `manifests/deploy.yaml` to use the local image with `imagePullPolicy: Never`, apply the manifest, wait for pod readiness via `kubectl wait -l app=tachyon-host`, and assert `GET /admin/status` responds 200.

#### Scenario: E2E workflow passes on a manifests-only PR
- **GIVEN** a PR modifies `manifests/deploy.yaml`
- **WHEN** the e2e-k8s job runs
- **THEN** the patched manifest is applied, the pod reaches Ready, and the healthcheck curl exits 0

### Requirement: Release workflow MUST publish server-binary tarballs on version tags
On `v*` tag pushes, `.github/workflows/release.yml` SHALL build and attach `tachyon-mesh-VERSION-OS-ARCH.tar.gz` tarballs for linux/x86_64, linux/aarch64, darwin/x86_64, and darwin/aarch64 to the GitHub release.

### Requirement: A one-command bootstrap script MUST exist for new contributors
The repository SHALL provide `scripts/setup.sh` (Linux/macOS) and `scripts/setup.ps1` (Windows) that check prerequisites (Rust, npm), add WASM targets, build core binaries and guest artifacts, install UI dependencies, run cross-layer validation, and print a success banner with startup commands and an MCP JSON snippet. Both scripts SHALL accept `--skip-guests` / `-SkipGuests` and `--skip-ui` / `-SkipUI` flags.

#### Scenario: Missing prerequisite exits with helpful message
- **GIVEN** `cargo` is not on PATH
- **WHEN** `./scripts/setup.sh` is run
- **THEN** the script exits with code 1 and prints the rustup install URL

#### Scenario: Idempotent re-run does not fail
- **GIVEN** setup has already been run once
- **WHEN** `./scripts/setup.sh` is run again
- **THEN** the script completes successfully (WASM target add and npm install are idempotent)

### Requirement: README Quick Start MUST lead with the bootstrap script
The `README.md` Quick Start section SHALL present `./scripts/setup.sh` (and the PowerShell equivalent) as the single first step, replacing the previous multi-command manual flow.

### Requirement: Playwright E2E tests MUST cover the critical auth-to-apply path
The `tachyon-ui` package SHALL include a Playwright test suite under `e2e/` that covers: (1) the credentials form rendering inside `auth-step-credentials` shadow DOM; (2) the app shell visibility after the `iam:authenticated` event; (3) the seal button visibility toggle on `config:staged`.

#### Scenario: Playwright locates shadow DOM inputs
- **GIVEN** Playwright navigates to the Vite dev server at port 1420
- **WHEN** the test resolves `tachyon-iam > auth-step-credentials > #cred-url`
- **THEN** the input is found and visible

#### Scenario: Seal button toggles on config:staged
- **GIVEN** the app shell is visible after a synthetic `iam:authenticated` event
- **WHEN** `config:staged` is dispatched on the window
- **THEN** `#btn-seal-apply` loses the `hidden` class

### Requirement: AppShell modal overlays MUST be managed by a standalone Web Component
All z-stack overlays (toast manager, guided tour, bundle conflict modal) SHALL be owned by `TachyonAppShellModalRoot` (`tachyon-app-shell-modal-root`). It SHALL listen to the `topology:conflict` window event and expose `openConflictModal`, `startTour`, and `startTourIfFirstVisit`.

#### Scenario: topology:conflict event opens the conflict modal
- **WHEN** `topology:conflict` fires with `{ conflicts: [...] }`
- **THEN** `TachyonAppShellModalRoot` calls `openConflictModal(conflicts)`
- **AND** the bundle conflict modal becomes visible


### Requirement: A TROUBLESHOOTING.md MUST cover the 15 most common failure modes
The repository SHALL contain a `TROUBLESHOOTING.md` in the root covering build failures (wasm target, MSVC, NASM), runtime errors (port conflict, integrity.lock signature, ONNX), UI errors (WebKitGTK), MCP errors (-32001, -32002, degraded schema), and Kubernetes/GPU issues (VRAM scheduling, GPU detection). `README.md` SHALL link to it.

### Requirement: trapFocus MUST support an Escape key onClose callback
`trapFocus(element, onClose?)` SHALL invoke `onClose` when the Escape key is pressed inside the trapped element, preventing default browser behaviour.

### Requirement: All modal dialogs MUST wire Escape to their close action
`TachyonIAM`, `TachyonAppShellModalRoot`, `TachyonBundleConflictModal`, and `TachyonUsersPanel` audit modal SHALL pass their close/dismiss callbacks to `trapFocus`.

### Requirement: Global loader MUST be announced by screen readers
The `#global-apply-loader` element SHALL carry `aria-live="polite"` and `aria-atomic="true"`. A `.sr-only` span with "Applying configuration, please wait…" SHALL be the first child; visual elements SHALL carry `aria-hidden="true"`.

### Requirement: KV result rendering MUST use DOM APIs not innerHTML
`TachyonStoragePanel.renderKvResult()` SHALL build the result zone using `createElement`, `textContent`, and `replaceChildren` so user-controlled namespace/key/value strings never pass through innerHTML.
