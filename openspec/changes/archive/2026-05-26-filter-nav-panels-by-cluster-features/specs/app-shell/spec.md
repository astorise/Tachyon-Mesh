## ADDED Requirements

### Requirement: App Shell nav MUST only render panels available on the cluster
The `<tachyon-app-shell-nav>` component SHALL filter the list of navigation routes using the `clusterFeaturesStore` before rendering, showing only routes whose `requires` feature flag is `true` or routes that have no `requires` constraint.

#### Scenario: Nav hides GPU panel when no GPU nodes enrolled
- **WHEN** `clusterFeaturesStore` reports `hasGpu: false`
- **THEN** the "AI Orchestration" nav button is not rendered in the sidebar

#### Scenario: Nav shows GPU panel when GPU nodes are present
- **WHEN** `clusterFeaturesStore` reports `hasGpu: true`
- **THEN** the "AI Orchestration" nav button is rendered in the sidebar

#### Scenario: Always-visible panels are never hidden
- **WHEN** `clusterFeaturesStore` reports any combination of features
- **THEN** the "Overview" and "Observability" nav buttons are always rendered

#### Scenario: Nav renders during feature loading
- **WHEN** `clusterFeaturesStore` has `status: "loading"` (features not yet fetched)
- **THEN** the nav renders only the always-visible panels (those with no `requires` constraint)

#### Scenario: Nav renders on store error
- **WHEN** `clusterFeaturesStore` has `status: "error"` and `features: null`
- **THEN** the nav renders only the always-visible panels

### Requirement: App Shell MUST redirect unavailable routes to overview
The App Shell router SHALL redirect the operator to the `overview` route when the active URL hash resolves to a panel whose required feature is not present in `clusterFeaturesStore`.

#### Scenario: Direct URL to unavailable panel redirects
- **WHEN** the operator navigates directly to `#ai` and `hasGpu` is `false`
- **THEN** the shell redirects to `#overview` without rendering the AI panel

#### Scenario: Direct URL to available panel is honoured
- **WHEN** the operator navigates directly to `#nodes` and `hasEnrolledNodes` is `true`
- **THEN** the shell renders the Nodes panel normally

#### Scenario: Direct URL to always-visible panel is always honoured
- **WHEN** the operator navigates directly to `#overview`
- **THEN** the shell renders the Overview panel regardless of cluster features
