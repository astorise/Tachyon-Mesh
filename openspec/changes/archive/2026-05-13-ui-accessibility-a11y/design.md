# Design: Accessibility (A11y) Foundation

## Approach

Five targeted, additive changes. No existing behaviour is altered. All ARIA additions are compatible with shadow-DOM encapsulation because attributes are set on elements inside the shadow root or on the host element itself.

### 1. Focus ring + utilities (`style.css`)

A `@layer base` block adds CSS `box-shadow` focus rings for `button`, `a`, `input`, `select`, and `textarea` on `:focus-visible`. This uses raw CSS rather than `@apply` (Tailwind v4 recommendation for base layers). The ring colour is `#3b82f6` (blue-500) with a `#1e293b` offset matching the slate-900 background.

Two new `@layer utilities` classes:
- `.sr-only` — visually hides content while keeping it in the accessibility tree. Used for input labels throughout.
- `.skip-nav` — off-screen skip link that appears on `:focus`. Positioned absolutely and jumps to `#main-content`.

### 2. Semantic layout (`TachyonAppShell.ts`)

The shell already used `<aside>`, `<nav>`, `<header>`, and `<main>`. Added attributes:
- `<aside aria-label="...">` — names the sidebar landmark for screen readers
- `<nav aria-label="...">` — distinguishes the navigation landmark from generic regions
- `<header aria-label="...">` — names the banner region
- `<main id="main-content" tabindex="-1" aria-label="...">` — `id` matches the skip-link href; `tabindex="-1"` allows programmatic focus when the skip link is used
- `<a class="skip-nav" href="#main-content">` — first element inside the shell for keyboard bypass
- The `id="router-view"` is renamed to `id="main-content"` to match the skip-link target

Six new i18n keys (`shell.skip-nav`, `shell.sidebar-label`, `shell.nav-label`, `shell.header-label`, `shell.main-label`, `shell.user-label`) in both EN and FR.

### 3. Form labeling (`TachyonIAM.ts`, `TachyonMfaPrompt.ts`)

Every visible `<input>` now has a corresponding `<label class="sr-only">` sibling and `aria-required="true"`. Key additions:
- Login form: `aria-label` on `<form>`, sr-only labels for URL/username/password, `autocomplete` hints
- MFA step: `aria-label` on `<form>`, sr-only label for the TOTP input, `autocomplete="one-time-code"`, `inputmode="numeric"`, `role="status"` on the prompt text
- `TachyonMfaPrompt`: `role="dialog"` / `aria-modal="true"` / `aria-labelledby="mfa-title"` on the `<dialog>` element (it was already a `<dialog>` tag, so native focus management applies); `role="alert"` on the error div

### 4. Live regions and modal accessibility

**`NetworkStatus.ts`** — `role="status"`, `aria-live="polite"`, `aria-atomic="true"` on the status root element. Screen readers announce connection state changes without interrupting the user.

**`TachyonBundleConflictModal.ts`** — `role="dialog"`, `aria-modal="true"`, `aria-labelledby="conflict-modal-title"` on the backdrop. The conflict list receives `aria-label` and the action group gets `aria-label`. Two new i18n keys added.

**Focus trap** — `trapFocus(container)` utility installed whenever the conflict modal renders with conflicts. It intercepts `Tab` / `Shift+Tab` and cycles focus within the modal's focusable elements. The cleanup function is stored in `removeFocusTrap` and called before each re-render and when the modal closes.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Focus ring CSS | Raw CSS `box-shadow` in `@layer base` | Tailwind `ring-*` via `@apply` | Tailwind v4 discourages `@apply` in `@layer base` |
| Input label strategy | `<label class="sr-only">` + `aria-required` | `aria-label` attribute only | sr-only labels are preferred by WCAG 2.1 SC 1.3.1; `aria-label` kept as redundant backup |
| Router-view rename | `router-view` → `main-content` | Keep old id, add new id | Skip links must match the fragment; single canonical id avoids duplication |
| Focus trap scope | Shadow root `role="dialog"` div | Full document body | The modal is in shadow DOM; document-level listeners can't reach shadow content |
