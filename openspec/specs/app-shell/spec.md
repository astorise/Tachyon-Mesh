# app-shell Specification

## Purpose
TBD - created by archiving change tachyon-ui-webcomponents-phase2. Update Purpose after archive.
## Requirements
### Requirement: Tachyon UI MUST expose the primary layout as an App Shell Web Component
The Tachyon UI SHALL render the primary sidebar, header, and router outlet through a native `<tachyon-app-shell>` custom element with Shadow DOM encapsulation.

#### Scenario: App shell starts hidden
- **GIVEN** the Tachyon UI page is loaded
- **WHEN** no IAM authentication event has been emitted
- **THEN** `<tachyon-app-shell>` remains visually hidden
- **AND** only the IAM layer is available to the operator.

### Requirement: App Shell MUST react to IAM authentication events
The App Shell component SHALL listen for `iam:authenticated`, transition into view with GSAP, and update its header with the authenticated user.

#### Scenario: Authentication reveals the shell
- **GIVEN** `<tachyon-iam>` emits `iam:authenticated`
- **WHEN** `<tachyon-app-shell>` receives the event
- **THEN** it hides the IAM layer
- **AND** it animates the sidebar, header, and router outlet into view
- **AND** the header displays the authenticated user.

### Requirement: App Shell MUST emit navigation events
The App Shell component SHALL emit `app:navigation` when an operator selects a sidebar route.

#### Scenario: Sidebar navigation emits a route
- **GIVEN** the App Shell is visible
- **WHEN** the operator clicks a sidebar route
- **THEN** the component emits `app:navigation`
- **AND** the event payload includes the selected route.

### Requirement: App Shell nav MUST only render panels available on the cluster
The `<tachyon-app-shell-nav>` component SHALL filter the list of navigation routes using the `clusterFeaturesStore` before rendering, showing only routes whose `requires` feature flag is `true` or routes that have no `requires` constraint.

#### Scenario: Nav hides panel when feature is absent
- **WHEN** `clusterFeaturesStore` reports `hasAi: false`
- **THEN** the "AI Orchestration" nav button is not rendered in the sidebar

#### Scenario: Nav shows panel when feature is present
- **WHEN** `clusterFeaturesStore` reports `hasAi: true`
- **THEN** the "AI Orchestration" nav button is rendered in the sidebar

#### Scenario: Always-visible panels are never hidden
- **WHEN** `clusterFeaturesStore` reports any combination of features
- **THEN** the "Overview" and "Dashboard" nav buttons are always rendered

#### Scenario: Nav renders during feature loading
- **WHEN** `clusterFeaturesStore` has `status: "loading"` (features not yet fetched)
- **THEN** the nav renders only the always-visible panels (those with no `requires` constraint)

#### Scenario: Nav renders on store error
- **WHEN** `clusterFeaturesStore` has `status: "error"` and `features: null`
- **THEN** the nav renders only the always-visible panels

### Requirement: App Shell MUST redirect unavailable routes to overview
The App Shell router SHALL redirect the operator to the `overview` route when the active URL hash resolves to a panel whose required feature is not present in `clusterFeaturesStore`.

#### Scenario: Direct URL to unavailable panel redirects
- **WHEN** the operator navigates directly to `#ai` and `hasAi` is `false`
- **THEN** the shell redirects to `#overview` without rendering the AI panel

#### Scenario: Direct URL to available panel is honoured
- **WHEN** the operator navigates directly to `#nodes` and `hasEnrolledNodes` is `true`
- **THEN** the shell renders the Nodes panel normally

#### Scenario: Direct URL to always-visible panel is always honoured
- **WHEN** the operator navigates directly to `#overview`
- **THEN** the shell renders the Overview panel regardless of cluster features

