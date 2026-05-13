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

