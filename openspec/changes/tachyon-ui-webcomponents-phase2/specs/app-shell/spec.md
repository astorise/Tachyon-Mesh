# Spec: Tachyon App Shell Web Component

## ADDED Requirements

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

## 1. Technical Identity
- **Tag Name**: `<tachyon-app-shell>`
- **Encapsulation**: `Shadow DOM (mode: 'open')`

## 2. Interface (Public API)

### Listened Events (Incoming)
- `iam:authenticated`: Listened to on the global `window` or `document`.
  - Action: Triggers the GSAP entrance timeline and populates the user profile in the Header.

### Emitted Events (Outgoing)
- `app:navigation`: Emitted when a user clicks a sidebar link.
  - Payload: `detail: { route: string }`

## 3. Internal Implementation

### Template Structure
The component generates a "Full Screen" layout hidden by default (`opacity: 0` or `display: none`).
Internal Shadow DOM structure:
1. **Sidebar (`<aside>`)**: Left navigation panel (bg-slate-800). Contains logo and links.
2. **Main Container (`<div class="flex-col">`)**:
   - **Header (`<header>`)**: Top bar displaying the username post-login.
   - **Content Area (`<main id="router-view">`)**: Scrollable container for dynamic route injection.

### Entrance Animation (GSAP)
Upon receiving `iam:authenticated`, the `show()` method orchestrates the following Timeline:
1. Hide the IAM layer (`#auth-layer` in the global DOM).
2. Set `<tachyon-app-shell>` to `display: flex`.
3. Animate Sidebar: `from x: -50, opacity: 0`.
4. Animate Header: `from y: -20, opacity: 0`.
5. Animate Content Area: `fade in`.
*(Note: Always target elements via `this.shadowRoot`)*.
