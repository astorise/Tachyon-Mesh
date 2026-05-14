# Technical Specification: Modals & Global Loaders

## 1. Focus Trap Utility (`tachyon-ui/src/utils/a11y.ts`)
Create a reusable focus trap for Vanilla Web Components.

```typescript
export function trapFocus(element: HTMLElement) {
  const focusableEls = element.querySelectorAll<HTMLElement>(
    'a[href], button, textarea, input[type="text"], input[type="radio"], input[type="checkbox"], select, [tabindex]:not([tabindex="-1"])'
  );
  const firstFocusableEl = focusableEls[0];
  const lastFocusableEl = focusableEls[focusableEls.length - 1];
  const KEYCODE_TAB = 9;

  element.addEventListener('keydown', function(e) {
    const isTabPressed = (e.key === 'Tab' || e.keyCode === KEYCODE_TAB);
    if (!isTabPressed) return;

    if (e.shiftKey) {
      if (document.activeElement === firstFocusableEl) {
        lastFocusableEl.focus();
        e.preventDefault();
      }
    } else {
      if (document.activeElement === lastFocusableEl) {
        firstFocusableEl.focus();
        e.preventDefault();
      }
    }
  });
  
  // Auto-focus first element when trap is initialized
  if (firstFocusableEl) firstFocusableEl.focus();
}
```

## 2. Modal Accessibility (`TachyonAppShellModalRoot.ts` & `TachyonIAM.ts`)
Update the container attributes and invoke the focus trap.

```html
<div 
  class="modal-backdrop fixed inset-0 z-[100] bg-black/50" 
  role="dialog" 
  aria-modal="true" 
  aria-labelledby="modal-title"
>
  </div>
```
*Implementation Note: Call `trapFocus(this.container)` immediately after rendering the modal HTML.*

## 3. Global Deploy Loader (`TachyonAppShell.ts`)
Wrap the `seal_and_apply` logic (lines ~327-403) with a global UI lock.

```typescript
async handleSealAndApply(manifest: any) {
  const mainContent = this.querySelector('#main-content') as HTMLElement;
  
  // 1. Lock UI
  mainContent.setAttribute('aria-busy', 'true');
  mainContent.classList.add('pointer-events-none', 'opacity-50', 'transition-opacity');
  
  // Create and append a global spinner overlay over the main content
  const loader = document.createElement('div');
  loader.id = 'global-apply-loader';
  loader.className = 'absolute inset-0 flex items-center justify-center z-50';
  loader.innerHTML = `<div class="spinner border-t-blue-500 border-4 rounded-full w-12 h-12 animate-spin"></div>`;
  mainContent.appendChild(loader);

  try {
    // ... existing seal_and_apply logic ...
  } finally {
    // 2. Unlock UI
    mainContent.removeAttribute('aria-busy');
    mainContent.classList.remove('pointer-events-none', 'opacity-50');
    const existingLoader = this.querySelector('#global-apply-loader');
    if (existingLoader) existingLoader.remove();
  }
}
```