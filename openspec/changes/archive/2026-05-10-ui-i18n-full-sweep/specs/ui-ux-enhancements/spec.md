# ui-ux-enhancements

## MODIFIED Requirements

### Requirement: Shell and overview text react to language changes
Every operator-visible string in all Tachyon-UI domain panels SHALL be
sourced from `utils/i18n.ts` rather than hardcoded. Each panel SHALL
subscribe to the `i18n:language-changed` event and re-render so the
language toggle takes immediate effect.

#### Scenario: Language toggle propagates to all domain panels
- **WHEN** the operator switches the shell language to `fr`
- **THEN** the headings, field labels, option text, placeholders, buttons,
  and feedback messages in AI, Hardware, Identity, RBAC, Workloads, Fleet,
  SupplyChain, and Resilience panels render in French
- **AND** the change takes effect without a page reload
