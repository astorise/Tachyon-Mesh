# Design: ui-component-decomposition

## Overview

Decomposed three responsibilities that had accumulated in `TachyonAppShell` and `TachyonIAM` into dedicated Web Components, each with a single clear purpose and clean event-based contracts.

## New Components

### `TachyonAuthStepCredentials` (`auth-step-credentials`)

Owns the login form: cluster URL, username, password, CA certificate upload/save/clear, and "remember me" checkbox.

**Contract:**
- Emits `credentials:submitted` (bubbles, composed) with `{ url, username, password, cert }` when the operator submits.
- Emits `credentials:url-changed` (bubbles, composed) with `{ url }` on every URL keystroke, allowing the parent to sync the signup URL field.
- Exposes `setUrl(url)` for programmatic pre-population.
- Exposes `persistIfRemember(url, username, password, cert)` for the parent to trigger Stronghold persistence after successful authentication.
- Calls `invoke("load_credentials")` in `connectedCallback` to self-restore saved credentials.
- Calls `invoke("load_custom_ca")` in `connectedCallback` to self-restore the saved CA certificate.

### `TachyonAppShellNav` (`tachyon-app-shell-nav`)

Owns the sidebar `<aside>` with all navigation links built from `listComponentRoutes()`.

**Contract:**
- Observes the `active-route` attribute; `attributeChangedCallback` re-renders and calls `updateActiveState()`.
- Emits `shell:navigate` (bubbles, composed) with `{ route }` on link click; also updates `window.location.hash` directly.
- Listens to `i18n:language-changed` to re-render labels in the current locale.

### `TachyonAppShellModalRoot` (`tachyon-app-shell-modal-root`)

Owns all overlay/z-stack elements: `<tachyon-guided-tour>`, `<tachyon-toast-manager>`, and `<tachyon-bundle-conflict-modal>`.

**Contract:**
- Listens to the `topology:conflict` window event and calls `openConflictModal()` automatically.
- Exposes `openConflictModal(conflicts)`, `startTour()`, and `startTourIfFirstVisit()` for parent orchestration.

## TachyonAppShell Changes

- Sidebar `<aside>` replaced with `<tachyon-app-shell-nav id="shell-sidebar" active-route="...">`.
- Modal section replaced with `<tachyon-app-shell-modal-root id="shell-modal-root">`.
- `updateNavigation()` now sets the `active-route` attribute on the nav element instead of toggling classes on individual buttons.
- `onTopologyConflict` handler removed; `TachyonAppShellModalRoot` owns that event subscription.
- `guidedTour()` private accessor replaced by `modalRoot()` which queries `tachyon-app-shell-modal-root`.
- Wires `shell:navigate` event listener on the nav component to sync the active route on hash-less programmatic navigation.

## TachyonIAM Changes

- `auth-step-login` form replaced with `<auth-step-credentials id="cred-step">` plus a standalone Register button.
- All CA-related logic (save, clear, restore, status display) removed — owned by `TachyonAuthStepCredentials`.
- All saved-credential restore logic removed — owned by `TachyonAuthStepCredentials.connectedCallback`.
- Listens to `credentials:submitted` to start the login flow; stores `stagedCredentials` for use in subsequent MFA/signup steps that need the cert.
- Listens to `credentials:url-changed` to keep the signup URL field in sync.
- Removed: `togglePassword`, `restoreSavedCredentials`, `restoreCustomCa`, `persistCredentialsPreference`, `saveSelectedCustomCa`, `clearCustomCa`, `currentCustomCa`, `readSelectedCert`, `updateCaStatus`, `syncAuthUrls`.

## Event Flow

```
auth-step-credentials
  └─ credentials:submitted ──► TachyonIAM.login()
  └─ credentials:url-changed ► TachyonIAM syncs signup URL input

tachyon-app-shell-nav
  └─ shell:navigate ──────────► TachyonAppShell.updateNavigation()
  └─ (sets window.location.hash → triggers hashchange)

window topology:conflict
  └─ TachyonAppShellModalRoot.onTopologyConflict
       └─ tachyon-bundle-conflict-modal.open()
```
