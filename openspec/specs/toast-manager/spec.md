# toast-manager Specification

## Purpose
Define the global toast notification system for Tachyon UI.

## Requirements
### Requirement: UI exposes a global toast manager
The Tachyon UI SHALL provide a `<tachyon-toast-manager>` web component that listens for global `app:notify` events and renders transient toast notifications outside the routed application shell.

#### Scenario: Toast manager is mounted at application root
- **WHEN** the Tachyon UI document is loaded
- **THEN** it includes `<tachyon-toast-manager>` after `<tachyon-app-shell>`
- **AND** the toast manager can render notifications without depending on the active route

#### Scenario: Notification event renders toast
- **WHEN** frontend code dispatches `app:notify` with `type` and `message` detail fields
- **THEN** the toast manager renders a toast with styling matching the type
- **AND** the toast auto-dismisses after a short delay

#### Scenario: Toast manager cleans up listener
- **WHEN** `<tachyon-toast-manager>` is disconnected
- **THEN** it removes its global event listener
- **AND** reconnecting does not register duplicate listeners

### Requirement: Dashboard feedback also emits global notifications
`TachyonConfigDashboard.showFeedback` SHALL continue updating the local `feedback-zone` and SHALL also dispatch an `app:notify` CustomEvent with the same feedback type and message.

#### Scenario: Dashboard success emits global notification
- **WHEN** a dashboard calls `showFeedback("success", message)`
- **THEN** the local feedback zone is updated
- **AND** a global `app:notify` event is dispatched with `type: "success"`

#### Scenario: Dashboard error emits global notification
- **WHEN** a dashboard calls `showFeedback("error", message)`
- **THEN** the local feedback zone is updated
- **AND** a global `app:notify` event is dispatched with `type: "error"`
