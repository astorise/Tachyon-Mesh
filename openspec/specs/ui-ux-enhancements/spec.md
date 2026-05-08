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
The Tachyon app shell and overview panel SHALL consume the i18n utility for user-facing navigation, header, dashboard, and overview telemetry labels.

#### Scenario: Shell language toggle updates visible shell text
- **WHEN** the operator changes the header language selector
- **THEN** the shell updates navigation, header, and dashboard labels without a page reload
- **AND** the current route remains selected

#### Scenario: Overview panel refreshes localized text
- **WHEN** the global `i18n:language-changed` event fires
- **THEN** `<tachyon-overview-panel>` re-renders its labels and telemetry descriptions in the active language
- **AND** it preserves the latest displayed metric values

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
