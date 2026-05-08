## Purpose

Define the cleanup requirements that remove the legacy Tachyon UI Light DOM application and unify routing under the Web Component shell.

## Requirements

### Requirement: Tachyon UI boots from a minimal host document
The Tachyon UI host document SHALL remove legacy Light DOM sidebars, dashboards, modals, and auth overlays, leaving only the Web Component bootstrap structure.

#### Scenario: Host body contains only bootstrap components
- **WHEN** `tachyon-ui/index.html` is loaded
- **THEN** the body contains `#app-root`
- **AND** `#app-root` contains `#auth-layer` with `<tachyon-iam>`
- **AND** `#app-root` contains `<tachyon-app-shell>`
- **AND** no legacy Light DOM sidebar, view container, or auth overlay remains

### Requirement: Legacy router is removed
The Tachyon UI SHALL remove the old imperative router module so route rendering is not split across two routing systems.

#### Scenario: Router module is absent
- **WHEN** the source tree is inspected
- **THEN** `tachyon-ui/src/router.ts` does not exist
- **AND** no bootstrap code instantiates the legacy `Router`

### Requirement: App shell owns hash routing through ComponentRegistry
The Tachyon app shell SHALL listen for URL hash changes and resolve routes through `ComponentRegistry`.

#### Scenario: Direct hash route opens registered component
- **WHEN** the URL hash is `#observability`
- **THEN** `<tachyon-app-shell>` resolves `observability` through `ComponentRegistry`
- **AND** renders the mapped Web Component in the router view

#### Scenario: Unknown hash falls back to dashboard
- **WHEN** the URL hash does not match `dashboard` or a route in `ComponentRegistry`
- **THEN** `<tachyon-app-shell>` renders the dashboard route

### Requirement: Main bootstrapper contains no legacy DOM controller
The Tachyon UI main entrypoint SHALL only import CSS, initialize global state/listeners, and register Web Components.

#### Scenario: Main entrypoint avoids Light DOM view orchestration
- **WHEN** `tachyon-ui/src/main.ts` is loaded
- **THEN** it does not query legacy navigation or view elements
- **AND** it does not manually toggle legacy route panels
- **AND** Web Component Shadow DOM owns UI rendering

### Requirement: Auth layer remains controlled from the shell transition
The Tachyon app shell SHALL hide the external `#auth-layer` after successful authentication.

#### Scenario: Authentication starts shell
- **WHEN** `<tachyon-iam>` emits `iam:authenticated`
- **THEN** `<tachyon-app-shell>` hides `#auth-layer`
- **AND** reveals the shell route selected by the current hash or dashboard fallback
