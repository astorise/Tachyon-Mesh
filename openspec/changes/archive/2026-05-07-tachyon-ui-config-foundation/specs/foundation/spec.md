# Spec: Tachyon UI Configuration Foundation

## ADDED Requirements

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
