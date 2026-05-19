## ADDED Requirements

### Requirement: Shared Policy Form badge component

Tachyon-UI SHALL ship a `<tachyon-policy-form-badge>` custom element registered in `tachyon-ui/src/components/base/TachyonPolicyFormBadge.ts`. The element MUST render a single visual chip with the i18n label `policy-form-badge.label` and MUST expose a tooltip whose copy comes from the i18n key `policy-form-badge.tooltip`.

#### Scenario: Badge renders the i18n label

- **WHEN** a host panel includes `<tachyon-policy-form-badge></tachyon-policy-form-badge>` in its header
- **THEN** the badge displays the localised text bound to `policy-form-badge.label`
- **AND** the element renders inside its own Shadow DOM with the shared stylesheet adopted

#### Scenario: Tooltip explains the absence of state

- **WHEN** the operator hovers the badge
- **THEN** the tooltip shows the localised text bound to `policy-form-badge.tooltip`
- **AND** the tooltip explicitly states that the host panel writes configuration and does not display the cluster's current state

#### Scenario: Language change re-renders the badge

- **GIVEN** the badge is mounted with the English locale
- **WHEN** an `i18n:language-changed` event is dispatched on `window`
- **THEN** the badge re-renders with the newly active locale's `policy-form-badge.label`
- **AND** the tooltip text re-renders likewise

### Requirement: Badge is applied to every policy-only panel

The following panels SHALL embed `<tachyon-policy-form-badge>` in their header, immediately next to the panel title:

- `<tachyon-resilience-panel>`
- `<tachyon-identity-panel>` (route `identity-config`)
- `<tachyon-rbac-panel>`
- `<tachyon-supply-chain-panel>`
- `<tachyon-fleet-panel>`

#### Scenario: Every policy panel exposes the badge

- **GIVEN** the application is mounted and authenticated
- **WHEN** the operator navigates to each of the five routes (`resilience`, `identity-config`, `rbac`, `supply-chain`, `fleet`)
- **THEN** the corresponding panel's header contains exactly one `<tachyon-policy-form-badge>` element
- **AND** the badge appears between the panel title and the right-hand side of the header

#### Scenario: Panels that display real state do NOT receive the badge

- **WHEN** the operator navigates to `overview`, `nodes`, `systems`, `topology`, `users`, `workloads`, `observability`, `storage`, or `ai`
- **THEN** the rendered panel header does NOT contain a `<tachyon-policy-form-badge>` element
- **AND** the absence of the badge is enforced by an assertion in the panel-level unit tests
