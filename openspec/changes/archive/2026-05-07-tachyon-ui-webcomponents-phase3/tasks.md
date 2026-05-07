# Tasks: Routing Dashboard Migration (Phase 3)

## Component Creation
- [x] Create `src/components/routing/TachyonRoutingDashboard.ts` and define the class extending `HTMLElement`.
- [x] Implement Tailwind Constructable Stylesheets injection.
- [x] Register the component using `customElements.define('tachyon-routing-dashboard', TachyonRoutingDashboard)`.

## Routing Form
- [x] Render L4 gateway and L7 route inputs inside the component Shadow DOM.
- [x] Build a strict `TrafficConfiguration` payload from input values.
- [x] Submit the payload to `apply_configuration` with the `config-routing` domain.

## App Shell Integration
- [x] Import the routing dashboard component in `TachyonAppShell`.
- [x] Add a routing panel in `#router-view`.
- [x] Show the routing panel when the `routing` sidebar item is selected.

## QA Validation
- [x] Verify successful validation emits `config:applied`.
- [x] Verify failed validation displays an inline error and emits `config:error`.
