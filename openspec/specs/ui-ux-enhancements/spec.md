## Purpose

Define the Tachyon UI localization and guided onboarding behavior for the Web Components shell.
## Requirements
### Requirement: Tachyon UI exposes a lightweight translation engine
The Tachyon UI SHALL provide a dependency-free i18n utility with English and French dictionaries, dot-notated keys, language persistence, and a global reactive language-change event.

#### Scenario: Language is changed at runtime
- **WHEN** a component calls `setLanguage("fr")`
- **THEN** the active language becomes French
- **AND** the selected language is persisted locally
- **AND** a global `i18n:language-changed` event is dispatched

#### Scenario: Translation lookup falls back safely
- **WHEN** a translation key is missing for the active language
- **THEN** the i18n utility returns the English value if available
- **AND** otherwise returns the requested key

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

### Requirement: Tachyon UI provides a guided tour component
The Tachyon UI SHALL provide a `<tachyon-guided-tour>` Web Component that explains key shell areas and uses GSAP to highlight target elements.

#### Scenario: Guided tour starts for first-time operators
- **WHEN** the authenticated shell starts and the tour has not been completed locally
- **THEN** `<tachyon-guided-tour>` opens automatically
- **AND** it stores completion state after the operator finishes or skips the tour

#### Scenario: Guided tour supports manual navigation
- **WHEN** the tour is open
- **THEN** it displays localized title and content for the current step
- **AND** it provides Previous, Next, Finish, and Skip controls
- **AND** GSAP animates the highlighted target and dialog

### Requirement: Shell header exposes language and tour controls
The Tachyon app shell header SHALL include a compact EN/FR selector and a Help/Tour trigger button.

#### Scenario: Operator switches language from the header
- **WHEN** the operator selects a language in the header selector
- **THEN** the shell calls the i18n language setter
- **AND** localized shell and overview labels update through the language-change event

#### Scenario: Operator launches tour from the header
- **WHEN** the operator clicks the Help/Tour button
- **THEN** the existing `<tachyon-guided-tour>` instance starts from the first step

### Requirement: Guided tour explains Atomic Seal & Apply
The Tachyon guided tour SHALL include an Atomic Seal & Apply step after the registry overview that targets the main pending Seal & Apply action.

#### Scenario: Operator reaches the Seal & Apply tour step
- **WHEN** the guided tour advances past the registry overview
- **THEN** it shows the title `Atomic Seal & Apply`
- **AND** it explains visual diffs, atomic signing, and Step-Up MFA
- **AND** it highlights the pending Seal & Apply action when available

### Requirement: Connection store persists MFA recency
The Tachyon UI connection store SHALL hydrate and persist `lastMfaTimestamp` through local browser storage so a page reload does not immediately discard a valid Step-Up MFA grace period.

#### Scenario: MFA timestamp is updated
- **WHEN** the UI records a new `lastMfaTimestamp`
- **THEN** the value is saved to local storage
- **AND** the in-memory connection store is updated

#### Scenario: UI reloads during MFA grace period
- **WHEN** the connection store initializes after a reload
- **THEN** it reads the stored timestamp
- **AND** uses `0` if storage is unavailable or invalid

### Requirement: Dynamic panel HTML escapes untrusted values
Tachyon UI panels that render dynamic values with `innerHTML` SHALL escape values from host data before interpolation.

#### Scenario: Topology source contains HTML
- **GIVEN** the topology source returned by the host contains HTML metacharacters
- **WHEN** `TachyonTopologyPanel` renders the source banner
- **THEN** those metacharacters are escaped before insertion into `innerHTML`

#### Scenario: User or group identifiers contain HTML
- **GIVEN** IAM user or group names contain HTML metacharacters
- **WHEN** `TachyonUsersPanel` renders table rows or action attributes
- **THEN** those identifiers are escaped before insertion into `innerHTML`

