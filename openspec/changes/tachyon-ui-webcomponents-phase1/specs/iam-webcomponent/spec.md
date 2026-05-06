# Spec: Tachyon IAM Web Component

## ADDED Requirements

### Requirement: Tachyon UI MUST expose IAM as an isolated Web Component
The Tachyon UI SHALL render the authentication and invite-token enrollment workflow through a native `<tachyon-iam>` custom element using an open Shadow DOM and component-scoped styles.

#### Scenario: IAM component boots in isolation
- **GIVEN** the Tachyon UI page is loaded
- **WHEN** the browser upgrades custom elements
- **THEN** `<tachyon-iam>` is defined and renders the login workflow inside its Shadow DOM
- **AND** the component styling does not depend on global DOM selectors.

### Requirement: IAM Web Component MUST publish authentication state via DOM events
The IAM component SHALL emit `iam:authenticated` after a successful Tauri-backed authentication or enrollment flow and SHALL emit `iam:error` for handled failures.

#### Scenario: Successful login emits authentication details
- **GIVEN** valid credentials are submitted in `<tachyon-iam>`
- **WHEN** the Rust backend accepts the authentication request
- **THEN** the component emits `iam:authenticated`
- **AND** the event payload includes the user, role, and token fields.

#### Scenario: Failed login is handled without panics
- **GIVEN** invalid credentials are submitted in `<tachyon-iam>`
- **WHEN** the backend rejects the request
- **THEN** the component displays an inline error
- **AND** the component emits `iam:error`.

## 1. Technical Identity
- **Tag Name**: `<tachyon-iam>`
- **Encapsulation**: `Shadow DOM` with `mode: 'open'` to allow unit testing and CSS injection.

## 2. Interface (Public API)

### Emitted Events (Outgoing)
- `iam:authenticated`: Emitted asynchronously as soon as the Rust backend (via Tauri) successfully validates the credentials.
  - Type: `CustomEvent`
  - Payload: `detail: { user: string, role: string, token: string }`
- `iam:error`: Emitted in case of rejection by the backend or a local network error.
  - Type: `CustomEvent`
  - Payload: `detail: { message: string, code: string }`

## 3. Internal Implementation

### Templates and DOM
- The component manages two internal templates: `login` (default) and `signup`.
- Template injection is done via `innerHTML` on the `shadowRoot`.
- Internal transitions between Login and Signup utilize `gsap`.

### Styling Management (Tailwind)
- Import `style.css` inline (via Vite/bundler configuration).
- Create a shared instance of `new CSSStyleSheet()`.
- Apply it to the `shadowRoot` via the `adoptedStyleSheets` property.

### Tauri/Rust Integration (Zero-Panic)
- `invoke('login', ...)` and `invoke('signup', ...)` are called from private methods within the class.
- The UI must never panic: any Rust error (`Result::Err`) must be cleanly formatted and passed to the `iam:error` dispatcher to be displayed within the component's UI (e.g., as a red warning text or toast).
