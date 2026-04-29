## ADDED Requirements

### Requirement: Tachyon UI tracks connection state in a global store
`tachyon-ui` SHALL maintain a global `useConnectionStore` (Zustand) that exposes a `CONNECTED`, `DISCONNECTED`, or `RECONNECTING` state and SHALL transition between these states based on the IPC/WebSocket transport status.

#### Scenario: Transport drop transitions UI to disconnected state
- **WHEN** the IPC or WebSocket connection to `core-host` is lost
- **THEN** the connection store transitions to `DISCONNECTED`
- **AND** components subscribed to the store receive the new state without page reloads
- **WHEN** the UI starts a reconnect attempt
- **THEN** the store transitions to `RECONNECTING`

### Requirement: UI reconnect uses exponential backoff
The UI SHALL retry the IPC/WebSocket connection using an exponential backoff schedule starting at one second and capped at thirty seconds.

#### Scenario: Multiple consecutive reconnect attempts back off
- **WHEN** the connection fails repeatedly
- **THEN** successive reconnect attempts wait 1s, 2s, 4s, 8s, 16s, then 30s between tries
- **AND** the cap of 30 seconds is preserved for any further attempts until reconnection succeeds

### Requirement: UI surfaces offline mode and supports optimistic updates for non-critical configuration
A non-intrusive global indicator SHALL be displayed while the connection is `DISCONNECTED` or `RECONNECTING`, and non-critical configuration changes SHALL be applied to local state optimistically and synced once the connection is restored.

#### Scenario: User edits a non-critical config while offline
- **WHEN** the UI is in `DISCONNECTED` state and the administrator edits a non-critical configuration field
- **THEN** the change is applied to local Zustand state immediately
- **AND** an offline indicator is visible in the UI
- **WHEN** the connection is restored
- **THEN** the queued change is sent to the host
- **AND** if the sync fails permanently, the local state is rolled back and the user is informed
