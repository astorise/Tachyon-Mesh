# Technical Specification: AppShell Decomposition

## 1. Component Extraction (`app-shell-nav`)
Create `tachyon-ui/src/components/layout/TachyonAppShellNav.ts`.
Extract the sidebar logic, domain links, and active state management from `TachyonAppShell.ts` into this new component.

```typescript
export class TachyonAppShellNav extends HTMLElement {
  // Handles DOM rendering for <nav>
  // Dispatches custom event: new CustomEvent('navigate', { detail: { domain: 'topology' } })
}
customElements.define('app-shell-nav', TachyonAppShellNav);
```

## 2. Component Extraction (`app-shell-modal-root`)
Create `tachyon-ui/src/components/layout/TachyonAppShellModalRoot.ts`.
Move all dialog, modal backdrop, and z-index orchestration logic into this isolated container.

```typescript
export class TachyonAppShellModalRoot extends HTMLElement {
  // Listens to global 'show-modal' and 'hide-modal' events.
  // Manages the <dialog> elements and focus trapping.
}
customElements.define('app-shell-modal-root', TachyonAppShellModalRoot);
```

## 3. Shell Orchestration
Refactor `TachyonAppShell.ts` to simply orchestrate these children.

```html
<div class="app-container">
  <app-shell-nav></app-shell-nav>
  <main id="main-content" tabindex="-1">
    </main>
  <app-shell-modal-root></app-shell-modal-root>
</div>
```